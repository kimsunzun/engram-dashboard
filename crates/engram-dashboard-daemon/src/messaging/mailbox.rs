//! mailbox — 인메모리 파킹 저장소(spec §5 · ADR-0103 결정 5 · ADR-0104 idle 게이트).
//!
//! ★역할★: 수신자별(이름 기반) FIFO 큐로 **아직 주입 못 한 메시지**를 잡아 둔다. 두 갈래가 같은 저장소·
//!   같은 `pending` 어휘를 공유한다(spec §5 분기 1 보정 — 새 상태 발명 금지):
//!     ① 부재 파킹 — 수신자가 미스폰·죽음·잠듦(unreachable). "없는 이름"도 파킹한다(스폰 전 선지시 지원,
//!        오타는 TTL 이 방어). wake 없음(v1 — ADR-0104).
//!     ② busy 대기 — 수신자는 살아 있으나 턴 진행 중이라 즉시 주입 못 함. idle 진입 때 일괄 flush.
//!   등장(스폰/epoch)·idle 진입 시 상위 서비스가 `drain` 으로 큐를 통째 비워 오래된 순으로 일괄 주입한다.
//!
//! ★불변식★:
//!   - **FIFO(오래된 순)** — `drain`/`sweep_expired` 는 park 순서를 보존한다(ADR-0104 일괄·오래된 순 flush).
//!   - **cap = 수신자당 100건, 초과 = 반려**(오래된 것 몰래 드롭 금지, spec §5 분기 3). 단 **notice 는 cap
//!     예외**(회신 계약의 타임아웃 통지가 가득 찬 메일박스에 막히면 계약이 반쪽 — spec §5 · ADR-0103 불변식).
//!   - **TTL = 1h** — 초과 항목은 `sweep_expired` 가 걷어내 상위가 장부에 `expired` 로 남긴다.
//!   - **순수 + 주입 시계** — 만료 판정은 `park` 시각과 인자 `now` 의 차로만 한다(모듈 헤더 순수성 불변식).
// ADR-0103
// ADR-0104

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// TTL — 파킹 항목의 최대 생존 기간. 초과분은 `sweep_expired` 가 걷어낸다.
///
/// ★왜 1h(spec §5 정책 상수)★: 오타·미스폰 수신자로 향한 메시지가 영원히 쌓이지 않게 하는 상한이다.
///   너무 짧으면 "스폰 전 선지시"(아직 안 뜬 에이전트 앞으로 미리 보냄)가 만료돼 버리고, 너무 길면 오배송이
///   오래 잔류한다 — 1h 는 그 절충(사용자 결정, ADR-0103). 상위 서비스가 sweep 주기를 정한다(여기선 값만).
const PARK_TTL: Duration = Duration::from_secs(60 * 60);

/// 수신자당 파킹 상한 — 초과 시 `MailboxFull` 반려(오래된 것 몰래 드롭 금지, spec §5 분기 3).
///
/// ★왜 100건(spec §5 정책 상수)★: 폭주하는 발신자가 한 수신자의 메일박스를 무한히 부풀려 메모리를 잠식하는
///   것을 막는 방어선이다. 초과를 조용히 drop-head 하면 유실이 은폐되므로(ADR-0103 거부 대안), 대신 신규를
///   즉시 반려해 발신자에게 가시화한다. **notice 는 이 cap 을 세지 않는다**(아래 `park` 의 예외 처리 참조).
const MAILBOX_CAP: usize = 100;

/// 파킹 항목의 종류 — cap 회계에서 notice 만 예외 취급하기 위한 최소 구분(spec §5).
///
/// ★notice 예외의 근거★: `<notice>`(데몬 전용 인프라 통지, 특히 request 타임아웃)는 메일박스가 가득 차도
///   반드시 발신자에게 도달해야 회신 계약이 성립한다(ADR-0103 불변식 "notice 는 메일박스 cap 예외 통로").
///   그래서 cap 검사에서 `Notice` 는 제외한다 — 큐 길이 상한과 무관하게 항상 park 된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkKind {
    /// 동료 발신(`<message>`) — cap 대상.
    Message,
    /// 데몬 통지(`<notice>`) — **cap 예외**(spec §5).
    Notice,
}

/// 파킹된 메시지 1건. 봉투 텍스트(주입 시 그대로 stdin 에 밀어넣을 완성 문자열)와 회계용 메타를 담는다.
///
/// ★envelope = 완성된 봉투 문자열★: 이 모듈은 순수 저장소라 봉투를 **조립하지 않는다** — 상위(ingress
///   wrap point)가 만든 완성 문자열을 그대로 보관·반환한다(단일 wrap point 불변식, ADR-0096). 여기선 그게
///   어떤 포맷인지 모른다(불투명 텍스트).
/// ★parked_at★: TTL 판정 기준 시각. 상위가 park 호출 시점의 `now` 를 주입한다(순수성 불변식).
#[derive(Debug, Clone)]
pub struct ParkedMessage {
    /// ledger 상관용 논리 메시지 id(상위가 부여). 저장소는 값으로만 나른다.
    pub msg_id: String,
    /// 완성된 봉투 문자열(주입 시 그대로 stdin 에 밀어넣음). 저장소는 조립하지 않는다(불투명).
    pub envelope: String,
    /// 항목 종류 — cap 예외 판정용(notice 는 cap 무시).
    pub kind: ParkKind,
    /// park 시각(주입된 now). TTL 판정 기준.
    pub parked_at: Instant,
}

impl ParkedMessage {
    /// `now` 기준으로 TTL(1h)에 도달했나. 경계(정확히 TTL)는 **만료**(`>=` 비교 — 아래 테스트 고정).
    ///
    /// ★경계 규약(load-bearing)★: `elapsed >= PARK_TTL` 이라 정확히 1h 가 지난 순간부터 만료다(경계 포함).
    ///   `>` 가 아니라 `>=` 를 쓰는 이유는 "TTL = 최대 생존 기간" 이라는 상한 의미와 정합하기 위함이다 —
    ///   1h 를 꽉 채운 항목은 더 살려 둘 이유가 없다(경계에서 즉시 만료가 상한 의미에 부합). 이 경계는 단위
    ///   테스트(`ttl_boundary_*`)가 고정한다 — 바꾸면 회귀.
    fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.parked_at) >= PARK_TTL
    }
}

/// park 반려 사유 — 상위가 wire 에러 코드로 매핑한다(현재 유일: cap 초과 → `MAILBOX_FULL`, spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkError {
    /// 수신자 큐가 cap(100)에 도달 — 신규 message 반려(notice 는 여기 도달 안 함). → `MAILBOX_FULL`.
    MailboxFull,
}

/// `drain` 결과 — 주입 가능분과 만료분을 **둘 다** 원자적으로 돌려준다(조용한 유실 금지, spec §5).
///
/// ★왜 두 컬렉션을 함께 반환하나(load-bearing — spec §5 "expired 장부 잔존")★: drain 이 만료분을 조용히
///   버리면 그 항목은 어디에도 기록되지 않아 유실이 은폐된다("조용한 유실 금지"). 그래서 drain 은 큐를
///   비우되(재-park 방지) 만료분도 함께 반환해, 상위가 **주입 가능분은 오래된 순으로 일괄 주입**하고
///   **만료분은 장부에 `expired` 로 남기게** 한다(경로가 아니라 반환값으로 유실을 막는다). `sweep_expired`
///   는 별도 주기적 청소 경로이고, drain 시점에 이미 만료된 것도 이 반환으로 반드시 장부화된다(두 경로 상보).
#[derive(Debug, Default)]
pub struct DrainOutcome {
    /// 미만료 = 오래된 순 일괄 주입 대상(상위가 그대로 stdin 에 밀어넣음).
    pub deliverable: Vec<ParkedMessage>,
    /// TTL 초과 = 주입 안 하고 장부에 `expired` 로 기록할 대상(오래된 순).
    pub expired: Vec<ParkedMessage>,
}

/// 수신자 이름(String, WYSIWYA — ADR-0101) → FIFO 큐 저장소. 부재 파킹·busy 대기 공용(spec §5).
///
/// ★순수★: 시간 의존 메서드는 전부 `now: Instant` 를 주입받는다(모듈 헤더 불변식). 내부에 시계 없음.
#[derive(Debug, Default)]
pub struct Mailbox {
    /// 수신자 이름별 FIFO 큐. `VecDeque` 는 push_back(park)·drain(앞에서부터) 모두 오래된 순 보존.
    queues: HashMap<String, std::collections::VecDeque<ParkedMessage>>,
}

impl Mailbox {
    /// 빈 메일박스.
    pub fn new() -> Self {
        Self::default()
    }

    /// 메시지를 수신자 큐 끝에 park(FIFO). cap 초과 = `MailboxFull` 반려(notice 는 cap 예외).
    ///
    /// ★cap 회계(spec §5)★: 큐의 **message 항목 수**가 cap 이상이면 신규 message 를 반려한다. notice 는
    ///   개수와 무관하게 항상 수용한다(회신 계약 통지가 막히면 안 됨 — ADR-0103 불변식). 그래서 cap 검사는
    ///   `kind == Message` 일 때만, 그리고 기존 **message 개수**만 센다(notice 는 분모에서도 제외).
    pub fn park(&mut self, recipient: &str, msg: ParkedMessage) -> Result<(), ParkError> {
        let queue = self.queues.entry(recipient.to_string()).or_default();
        // notice 는 cap 예외 — message 만 상한 검사(분모도 message 만 센다).
        if msg.kind == ParkKind::Message {
            let message_count = queue.iter().filter(|m| m.kind == ParkKind::Message).count();
            if message_count >= MAILBOX_CAP {
                return Err(ParkError::MailboxFull);
            }
        }
        queue.push_back(msg);
        Ok(())
    }

    /// ★재파킹(무손실 복원) primitive — cap 우회 + FRONT 삽입(ADR-0103/0104 · finding 1)★: flush 배치
    ///   도중 inject 실패로 아직 배달 못 한 **이미 admitted 된** 항목들을 큐 **앞쪽**에 원래 순서로 되돌린다.
    ///
    /// ★왜 `park` 가 아니라 별도 primitive 인가(load-bearing — 조용한 유실 금지)★: `park` 는 **신규
    ///   admission** 통제라 cap 을 세고 초과 시 반려한다. 그런데 재파킹은 이미 장부에 `pending` 으로 들어간
    ///   (admitted) 항목의 **보류 복원**이지 신규 발송이 아니다 — cap 은 유입 통제일 뿐 **보관 통제가 아니다**.
    ///   drain↔inject 사이 동시 park 로 큐가 다시 cap 까지 찼을 때 `park` 로 되돌리면 `MailboxFull` 이 나고,
    ///   그 에러를 무시하면 admitted 메시지가 조용히 유실된다(ledger 는 pending 인데 큐엔 없음 — 유령 pending).
    ///   그래서 재파킹은 cap 을 **우회**한다(보관은 무제한 — 유입만 cap). 상한 위반이 걱정되면 그건 유입
    ///   경로(`park`)가 이미 막고 있고, 재파킹분은 원래 그 cap 안에서 admitted 됐던 것이다.
    /// ★왜 FRONT 삽입인가(FIFO 역전 방지 — finding 1)★: 재파킹분은 동시 park 된 신규분보다 **먼저** 파킹된
    ///   더 오래된 항목이다. 큐 뒤(push_back)에 붙이면 신규분(더 최근)이 앞서게 돼 "오래된 순" 이 깨진다.
    ///   그래서 원래 순서 그대로 큐 **앞**에 되꽂아, 이후 drain 이 여전히 오래된 순(재파킹분 → 동시 park 분)을
    ///   낸다. `parked_at` 은 **원래 값 유지**(호출자가 원본 ParkedMessage 를 그대로 넘김) — TTL 이 연장되지
    ///   않는다(오배송 방어).
    /// ★notice/message 무관 무조건 수용★: 재파킹은 kind 를 안 본다(이미 admitted). cap 회계는 신규 park 만.
    pub fn restore_front(&mut self, recipient: &str, items: Vec<ParkedMessage>) {
        if items.is_empty() {
            return;
        }
        let queue = self.queues.entry(recipient.to_string()).or_default();
        // 원래 순서 보존하며 앞쪽에 되꽂기: 역순으로 push_front 하면 최종 순서가 items 순서 그대로 앞에 온다
        //   (마지막 항목을 먼저 push_front → 그 앞에 이전 항목 → … → 첫 항목이 최종 front).
        for item in items.into_iter().rev() {
            queue.push_front(item);
        }
    }

    /// 수신자 큐를 통째로 비워 주입 가능분·만료분을 **둘 다** 오래된 순으로 반환(flush primitive).
    ///
    /// ★왜 만료분도 반환하나(조용한 유실 금지 — spec §5)★: idle 진입/등장 flush 시 이미 TTL 지난 메시지를
    ///   주입하면 안 되지만(그건 `expired` 로 갈 몫), 그렇다고 **버리면 유실이 은폐된다**. 그래서 drain 은
    ///   큐를 비우되(재-park 방지) 만료분을 `expired` 로 함께 돌려줘, 상위가 그것을 장부에 `expired` 로
    ///   남기게 한다("expired 장부 잔존"). 주입은 `deliverable`(미만료, 오래된 순)만 한다(ADR-0104 일괄 flush).
    /// ★큐 제거★: 비운 뒤 빈 큐는 맵에서 없앤다(빈 이름이 무한 누적되지 않게).
    pub fn drain(&mut self, recipient: &str, now: Instant) -> DrainOutcome {
        let Some(queue) = self.queues.remove(recipient) else {
            return DrainOutcome::default();
        };
        // FIFO(오래된 순) 유지: VecDeque 순회 순서 = park 순서. 만료/미만료로 가르되 둘 다 순서 보존.
        let mut outcome = DrainOutcome::default();
        for m in queue {
            if m.is_expired(now) {
                outcome.expired.push(m);
            } else {
                outcome.deliverable.push(m);
            }
        }
        outcome
    }

    /// TTL 초과 항목을 **전 수신자에 걸쳐** 걷어내 반환한다(상위가 각 항목을 장부에 `expired` 로 기록).
    ///
    /// ★반환 = 걷어낸 만료분(오래된 순, 수신자 무관 평탄화)★: 상위는 이 목록을 순회하며 ledger 에 expired
    ///   전이를 남긴다(spec §5 "TTL 초과 expired, 장부 잔존"). 비워진 큐는 맵에서 제거한다.
    /// ★순수★: 만료 판정은 인자 `now` 로만 한다(모듈 헤더 불변식).
    pub fn sweep_expired(&mut self, now: Instant) -> Vec<ParkedMessage> {
        let mut expired = Vec::new();
        // 큐를 순회하며 만료분을 앞에서부터 분리(오래된 순 유지). 비면 제거.
        self.queues.retain(|_recipient, queue| {
            // 오래된 순 보존: 만료(front 쪽에 몰림)를 앞에서부터 뽑는다.
            while let Some(front) = queue.front() {
                if front.is_expired(now) {
                    // pop_front 는 위 front 존재 확인 직후라 항상 Some.
                    expired.push(queue.pop_front().expect("front 존재 확인 직후"));
                } else {
                    // FIFO 라 front 가 안 만료면 뒤도 안 만료(더 최근) — 조기 종료.
                    break;
                }
            }
            !queue.is_empty()
        });
        expired
    }

    /// 수신자 큐의 현재 항목 수(테스트·관측용). 큐가 없으면 0.
    pub fn len(&self, recipient: &str) -> usize {
        self.queues.get(recipient).map(|q| q.len()).unwrap_or(0)
    }

    /// 저장소 전체가 비었나(전 수신자 큐 없음). 관측/테스트용.
    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 파킹 항목 생성 — id·kind·park 시각을 지정한다.
    fn parked(id: &str, kind: ParkKind, at: Instant) -> ParkedMessage {
        ParkedMessage {
            msg_id: id.to_string(),
            envelope: format!("<message>{id}</message>"),
            kind,
            parked_at: at,
        }
    }

    #[test]
    fn park_and_drain_preserves_fifo_order() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..5 {
            mb.park("alice", parked(&format!("m{i}"), ParkKind::Message, t0))
                .expect("park 성공");
        }
        let drained = mb.drain("alice", t0);
        let ids: Vec<&str> = drained
            .deliverable
            .iter()
            .map(|m| m.msg_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["m0", "m1", "m2", "m3", "m4"],
            "drain 은 park 순서(오래된 순)를 보존해야"
        );
        assert!(drained.expired.is_empty(), "만료 없음");
    }

    #[test]
    fn drain_empties_the_queue() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park("alice", parked("m0", ParkKind::Message, t0))
            .unwrap();
        assert_eq!(mb.len("alice"), 1);
        let _ = mb.drain("alice", t0);
        assert_eq!(mb.len("alice"), 0, "drain 후 큐가 비어야");
        assert!(mb.is_empty(), "빈 큐는 맵에서 제거돼야");
    }

    #[test]
    fn drain_absent_recipient_is_empty() {
        let mut mb = Mailbox::new();
        let drained = mb.drain("nobody", Instant::now());
        assert!(
            drained.deliverable.is_empty(),
            "없는 수신자 drain 은 빈 목록"
        );
        assert!(drained.expired.is_empty());
    }

    #[test]
    fn cap_rejects_message_beyond_100() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // 정확히 cap(100)까지는 수용.
        for i in 0..MAILBOX_CAP {
            mb.park("bob", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째(cap 이내)는 수용해야"));
        }
        assert_eq!(mb.len("bob"), MAILBOX_CAP);
        // 101번째 message 는 반려.
        let over = mb.park("bob", parked("overflow", ParkKind::Message, t0));
        assert_eq!(
            over,
            Err(ParkError::MailboxFull),
            "cap 초과 message 는 MailboxFull 반려"
        );
        assert_eq!(mb.len("bob"), MAILBOX_CAP, "반려된 항목은 큐에 안 들어감");
    }

    #[test]
    fn notice_is_exempt_from_cap_beyond_100() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // 큐를 message 로 cap 까지 채운다.
        for i in 0..MAILBOX_CAP {
            mb.park("carol", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        // message 는 이제 반려되지만…
        assert_eq!(
            mb.park("carol", parked("msg-over", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull)
        );
        // …notice 는 cap 을 무시하고 계속 park 돼야(회신 계약 통지 보장, spec §5).
        for i in 0..3 {
            mb.park("carol", parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("notice 는 cap 예외여야({i})"));
        }
        assert_eq!(
            mb.len("carol"),
            MAILBOX_CAP + 3,
            "notice 3건이 cap 위에 얹혀야"
        );
        // notice 로 큐가 넘쳐도 여전히 신규 message 는 반려(분모는 message 만).
        assert_eq!(
            mb.park("carol", parked("msg-over2", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull),
            "notice 가 message 분모를 부풀리면 안 됨"
        );
    }

    #[test]
    fn ttl_boundary_exactly_at_ttl_is_expired() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park("d", parked("m0", ParkKind::Message, t0)).unwrap();
        // 정확히 TTL 인 순간 = 만료(`>=` 경계, is_expired 규약). drain 은 deliverable 이 아닌 expired 로 낸다.
        let at_ttl = t0 + PARK_TTL;
        let drained = mb.drain("d", at_ttl);
        assert!(
            drained.deliverable.is_empty(),
            "정확히 TTL 인 순간은 만료(경계 포함) — deliverable 아님"
        );
        assert_eq!(
            drained.expired.len(),
            1,
            "정확히 TTL 인 순간은 expired 로 표면화"
        );
        assert_eq!(drained.expired[0].msg_id, "m0");
    }

    #[test]
    fn ttl_boundary_just_over_ttl_is_expired() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park("d", parked("m0", ParkKind::Message, t0)).unwrap();
        // TTL 을 1ns 초과 = 만료 → drain 은 deliverable 이 아닌 expired 로 낸다(조용한 유실 금지).
        let over = t0 + PARK_TTL + Duration::from_nanos(1);
        let drained = mb.drain("d", over);
        assert!(
            drained.deliverable.is_empty(),
            "TTL 초과분은 주입 대상 아님"
        );
        assert_eq!(drained.expired.len(), 1, "TTL 초과분은 expired 로 표면화");
    }

    #[test]
    fn drain_surfaces_expired_never_silently_dropped() {
        // 조용한 유실 금지(spec §5): 큐에 만료+미만료가 섞여도 만료분은 버려지지 않고 expired 로 나온다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // old 는 만료되게(t0), recent 는 살아 있게(t0 + 30m). now = t0 + 1h.
        mb.park("h", parked("old", ParkKind::Message, t0)).unwrap();
        mb.park(
            "h",
            parked("recent", ParkKind::Message, t0 + Duration::from_secs(1800)),
        )
        .unwrap();
        let now = t0 + PARK_TTL;
        let drained = mb.drain("h", now);
        let deliverable: Vec<&str> = drained
            .deliverable
            .iter()
            .map(|m| m.msg_id.as_str())
            .collect();
        let expired: Vec<&str> = drained.expired.iter().map(|m| m.msg_id.as_str()).collect();
        assert_eq!(deliverable, vec!["recent"], "미만료만 주입 대상");
        assert_eq!(
            expired,
            vec!["old"],
            "만료는 조용히 사라지지 않고 expired 로"
        );
        assert!(mb.is_empty(), "drain 은 큐를 비운다(재-park 방지)");
    }

    #[test]
    fn sweep_expired_removes_and_returns_expired_only() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // 오래된 것(t0) + 최근 것(t0 + 30m). now = t0 + 1h + 1ns 면 오래된 것만 만료.
        mb.park("e", parked("old", ParkKind::Message, t0)).unwrap();
        mb.park(
            "e",
            parked("recent", ParkKind::Message, t0 + Duration::from_secs(1800)),
        )
        .unwrap();
        let now = t0 + PARK_TTL + Duration::from_nanos(1);
        let expired = mb.sweep_expired(now);
        assert_eq!(expired.len(), 1, "만료분 1건만 반환");
        assert_eq!(expired[0].msg_id, "old");
        assert_eq!(mb.len("e"), 1, "최근 것은 큐에 잔존");
        // recent 만 남았으니 drain 하면 그것(now 는 recent 기준 미만료).
        let rest = mb.drain("e", now);
        assert_eq!(rest.deliverable[0].msg_id, "recent");
        assert!(rest.expired.is_empty());
    }

    #[test]
    fn sweep_expired_removes_empty_queues() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park("f", parked("only", ParkKind::Message, t0)).unwrap();
        let now = t0 + PARK_TTL + Duration::from_nanos(1);
        let expired = mb.sweep_expired(now);
        assert_eq!(expired.len(), 1);
        assert!(mb.is_empty(), "전부 만료돼 비면 큐가 맵에서 제거돼야");
    }

    // ── restore_front(재파킹 무손실 복원 — finding 1) ────────────────────────────────
    #[test]
    fn restore_front_bypasses_cap_no_loss() {
        // ★조용한 유실 금지(ADR-0103 · finding 1)★: 큐가 이미 cap(100) 이면 park 는 반려하지만, restore_front
        //   는 cap 을 우회해 admitted 항목을 무조건 되돌린다(유령 pending 방지).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park("r", parked(&format!("new{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        assert_eq!(mb.len("r"), MAILBOX_CAP);
        // park 는 이제 반려된다(cap 도달).
        assert_eq!(
            mb.park("r", parked("would-reject", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull)
        );
        // restore_front 는 cap 을 넘어서라도 되돌린다(유실 0).
        let older = vec![
            parked("old0", ParkKind::Message, t0),
            parked("old1", ParkKind::Message, t0),
        ];
        mb.restore_front("r", older);
        assert_eq!(
            mb.len("r"),
            MAILBOX_CAP + 2,
            "재파킹은 cap 을 우회해 admitted 항목을 되돌린다(유실 0)"
        );
    }

    #[test]
    fn restore_front_preserves_oldest_first_before_concurrent_parks() {
        // ★FIFO 역전 방지(finding 1)★: 재파킹분(더 오래됨)은 동시 park 된 신규분보다 앞서야 한다.
        //   시나리오: drain 으로 [old0,old1,old2] 를 꺼냈고 그 사이 new0·new1 이 park 됐다 → old0 배달 후
        //   실패 → [old1,old2] 재파킹. 이때 큐 = [new0,new1] 이므로 restore_front 후 = [old1,old2,new0,new1].
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // drain↔inject 사이 동시 park 된 신규분(더 최근).
        mb.park(
            "r",
            parked("new0", ParkKind::Message, t0 + Duration::from_secs(10)),
        )
        .unwrap();
        mb.park(
            "r",
            parked("new1", ParkKind::Message, t0 + Duration::from_secs(11)),
        )
        .unwrap();
        // 재파킹할 오래된 항목(원래 순서 유지).
        let older = vec![
            parked("old1", ParkKind::Message, t0 + Duration::from_secs(1)),
            parked("old2", ParkKind::Message, t0 + Duration::from_secs(2)),
        ];
        mb.restore_front("r", older);
        // drain 하면 재파킹분(오래됨) → 동시 park 분(최근) 순서.
        let drained = mb.drain("r", t0 + Duration::from_secs(20));
        let ids: Vec<&str> = drained
            .deliverable
            .iter()
            .map(|m| m.msg_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["old1", "old2", "new0", "new1"],
            "재파킹분이 동시 park 신규분보다 앞서야(오래된 순 보존)"
        );
    }

    #[test]
    fn restore_front_empty_is_noop() {
        let mut mb = Mailbox::new();
        mb.restore_front("r", Vec::new());
        assert!(mb.is_empty(), "빈 재파킹은 큐를 만들지 않는다");
    }

    #[test]
    fn sweep_preserves_oldest_first_within_recipient() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // 세 건 모두 만료되게(오래된 순으로 park).
        mb.park("g", parked("first", ParkKind::Message, t0))
            .unwrap();
        mb.park(
            "g",
            parked("second", ParkKind::Message, t0 + Duration::from_secs(1)),
        )
        .unwrap();
        mb.park(
            "g",
            parked("third", ParkKind::Message, t0 + Duration::from_secs(2)),
        )
        .unwrap();
        let now = t0 + PARK_TTL + Duration::from_secs(10);
        let expired = mb.sweep_expired(now);
        let ids: Vec<&str> = expired.iter().map(|m| m.msg_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["first", "second", "third"],
            "sweep 도 오래된 순 보존"
        );
    }
}
