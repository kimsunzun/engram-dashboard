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
//!   - **큐 정렬축 = admission 순번(`ParkedMessage.admission_seq`)** — 큐는 앞→뒤로 순번이 **강한 증가**다
//!     (`park` 는 새 순번을 뒤에 붙이고, `restore_ordered` 는 순번 기준 merge 로 되꽂는다). "오래된 순" 의
//!     정본 축이 시계가 아니라 이 순번인 이유는 `admission_seq` 주석 참조(round-4 finding 1).
//!   - **cap = 수신자당 100건, 초과 = 반려**(오래된 것 몰래 드롭 금지, spec §5 분기 3). 단 **notice 는 cap
//!     예외**(회신 계약의 타임아웃 통지가 가득 찬 메일박스에 막히면 계약이 반쪽 — spec §5 · ADR-0103 불변식).
//!   - **TTL = 24h** — 초과 항목은 `sweep_expired` 가 걷어내 상위가 장부에 `expired` 로 남긴다(ADR-0105 —
//!     1h → 24h 상향, 인메모리 단계 한정).
//!   - **순수 + 주입 시계** — 만료 판정은 `park` 시각과 인자 `now` 의 차로만 한다(모듈 헤더 순수성 불변식).
// ADR-0103
// ADR-0104
// ADR-0105

use std::collections::HashMap;
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::types::AgentId;

/// TTL — 파킹 항목의 최대 생존 기간. 초과분은 `sweep_expired` 가 걷어낸다.
///
/// ★왜 24h(spec §5 정책 상수, ADR-0105 — 1h 에서 상향)★: 선례 조사(/research light, 2026-07-25) 상
///   업계 관행이 일 단위다(SQS 4일·Kafka 7일·Postfix 5일 — 1h 는 그 대비 이례적으로 짧았다). "살아있는
///   수신자는 TTL 면제"(liveness-aware) 는 조사한 6개 시스템(RabbitMQ·SQS·Kafka·Postfix·ejabberd·LLM
///   프레임워크) 어디에도 선례가 없어 채택하지 않는다 — 부재든 busy 대기든 시계 기반 단일 규칙을 그대로
///   유지한다. 인메모리 단계 + cap 100(아래 `MAILBOX_CAP`) 이라 긴 TTL 의 비용은 ~0(데몬 재시작 시 전부
///   소멸) — **영속화(디스크) 단계가 오면 재설계 전제**(사용자 결정, 2026-07-25). 상위 서비스가 sweep
///   주기를 정한다(여기선 값만).
// ADR-0105
const PARK_TTL: Duration = Duration::from_secs(24 * 60 * 60);

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
    /// ★admission 순번 — 큐 정렬축(round-4 finding 1)★: `park` 이 수용 시점에 부여하는 단조 증가 번호다.
    ///   **호출자가 넣은 값은 무시·덮어쓴다**(저장소가 유일한 부여자 — 두 부여자가 있으면 순서가 갈린다).
    ///
    /// ★왜 `parked_at`(시계)이 아니라 별도 순번인가(load-bearing)★: 재파킹 merge 와 만료 판정은 **서로 다른
    ///   축**이다. 만료는 시계(`parked_at`)로 봐야 하지만, "누가 먼저 큐에 들어왔나" 를 시계로 판정하면 두
    ///   군데서 틀어진다: ① `park_pending` 은 락 **획득 전에** `Instant::now()` 를 뜨므로 두 발송이 경합하면
    ///   시각과 실제 수용 순서가 역전될 수 있다 ② 시계 분해능이 거친 환경에선 연속 park 의 `parked_at` 이
    ///   **같은 값**이 돼 순서가 결정 불가다. 순번은 저장소 안에서 락 보유 중 부여되므로 두 문제 모두 없다 —
    ///   그래서 `restore_ordered` 의 merge 키는 순번이고, 큐는 항상 순번 강한 증가다.
    pub admission_seq: u64,
    /// ★해석된 수신자 id 힌트(있을 때만 — C2 리뷰 fix 2)★: 이 메시지를 park 할 때 발송이 **구체적인 산
    ///   수신자를 이미 해석했다면** 그 AgentId. busy 대기·주입 실패 파킹은 항상 값이 있고, 부재 파킹
    ///   ("없는 이름" 앞 선지시)은 `None` 이다.
    ///
    /// ★왜 필요한가(이름-키 파킹의 사각지대)★: 파킹의 주소 단위는 **이름**이다(respawn 생존 —
    ///   근거는 service.rs `canonical_park_key`). 그런데 flush 는 "그 이름의 도달 후보가 **정확히 1개**" 일 때만 배달한다(동명
    ///   다수는 보류). 여기서 구멍이 생긴다: exact-AgentId 로 지목한 발송은 동명 모호성을 **의도적으로
    ///   통과**하는데(id 가 명시적 승자), 그 수신자가 turn 중이라 이름-키로 park 되면 동명이 둘인 동안
    ///   flush 가 영영 보류돼 TTL 까지 blackhole 이 된다. 그래서 park 시점에 해석된 id 를 힌트로 함께
    ///   보관해, flush 가 **그 id 가 아직 살아 있으면 이름 유일성과 무관하게** 그쪽으로 배달한다.
    /// ★힌트는 권위가 아니라 우선순위다★: 그 id 가 죽었으면(재스폰 등) 무시하고 이름 규칙으로 되돌아간다
    ///   — 그래서 "재스폰된 동명이 파킹을 이어받는다" 는 이름-키 설계가 그대로 유지된다.
    pub hinted_id: Option<AgentId>,
}

impl ParkedMessage {
    /// `now` 기준으로 TTL 에 도달했나. 경계(정확히 TTL)는 **만료**(`>=` 비교 — 아래 테스트 고정).
    ///
    /// ★경계 규약(load-bearing)★: `elapsed >= PARK_TTL` 이라 정확히 TTL 이 지난 순간부터 만료다(경계 포함).
    ///   `>` 가 아니라 `>=` 를 쓰는 이유는 "TTL = 최대 생존 기간" 이라는 상한 의미와 정합하기 위함이다 —
    ///   TTL 을 꽉 채운 항목은 더 살려 둘 이유가 없다(경계에서 즉시 만료가 상한 의미에 부합). 이 경계는 단위
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
    /// 다음 admission 순번(전 수신자 공유 단조 카운터 — `ParkedMessage.admission_seq` 부여자).
    ///   수신자별이 아니라 저장소 전역인 이유: 한 이름 큐에 여러 타깃 몫이 섞여도(동명 다수) 순번 하나로
    ///   전역 수용 순서를 표현할 수 있고, u64 라 실질적으로 소진되지 않는다.
    next_seq: u64,
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
    pub fn park(&mut self, recipient: &str, mut msg: ParkedMessage) -> Result<(), ParkError> {
        let queue = self.queues.entry(recipient.to_string()).or_default();
        // notice 는 cap 예외 — message 만 상한 검사(분모도 message 만 센다).
        if msg.kind == ParkKind::Message {
            let message_count = queue.iter().filter(|m| m.kind == ParkKind::Message).count();
            if message_count >= MAILBOX_CAP {
                return Err(ParkError::MailboxFull);
            }
        }
        // admission 순번은 **수용이 확정된 뒤** 저장소가 부여한다(반려분은 번호를 태우지 않는다). 호출자 값은
        //   덮어쓴다 — 부여자가 여기 하나여야 큐의 "순번 강한 증가" 불변식이 성립한다.
        msg.admission_seq = self.next_seq;
        self.next_seq += 1;
        queue.push_back(msg);
        Ok(())
    }

    /// ★재파킹(무손실 복원) primitive — cap 우회 + admission 순번 merge(ADR-0103/0104 · finding 1)★: flush
    ///   배치 도중 배달하지 못한 **이미 admitted 된** 항목들을, 큐의 나머지와 **전역 수용 순서대로 섞어**
    ///   되돌린다(단순 앞쪽 삽입이 아니다 — 아래 "왜 merge 인가").
    ///
    /// ★왜 `park` 가 아니라 별도 primitive 인가(load-bearing — 조용한 유실 금지)★: `park` 는 **신규
    ///   admission** 통제라 cap 을 세고 초과 시 반려한다. 그런데 재파킹은 이미 장부에 `pending` 으로 들어간
    ///   (admitted) 항목의 **보류 복원**이지 신규 발송이 아니다 — cap 은 유입 통제일 뿐 **보관 통제가 아니다**.
    ///   drain↔inject 사이 동시 park 로 큐가 다시 cap 까지 찼을 때 `park` 로 되돌리면 `MailboxFull` 이 나고,
    ///   그 에러를 무시하면 admitted 메시지가 조용히 유실된다(ledger 는 pending 인데 큐엔 없음 — 유령 pending).
    ///   그래서 재파킹은 cap 을 **우회**한다(보관은 무제한 — 유입만 cap). 상한 위반이 걱정되면 그건 유입
    ///   경로(`park`)가 이미 막고 있고, 재파킹분은 원래 그 cap 안에서 admitted 됐던 것이다.
    /// ★왜 단순 FRONT 삽입이 아니라 merge 인가(전역 오래된 순 — round-4 finding 1)★: 한 flush 는 같은 이름
    ///   큐에 대해 **재파킹을 여러 번** 부를 수 있다 — ① busy/도달불가 스킵분(락 안, 배치 시작 전) ② 타깃별
    ///   inject 실패분(락 밖, 타깃마다 따로). 한 이름 큐에 동명 다수의 몫이 섞여 있으면(exact-id 발송 + 동명
    ///   respawn) 이 호출들이 **각각** 앞쪽에 꽂히는데, 그러면 나중 호출이 앞선 호출보다 앞에 놓여 호출 간
    ///   나이 순서가 **뒤집힌다**(예: A 몫 [m0,m2] 복원 → B 몫 [m1,m3] 복원 = [m1,m3,m0,m2]). 그래서 앞쪽
    ///   삽입 대신 **admission 순번 기준 merge** 로 되꽂아, 몇 번을 부르든 큐가 항상 전역 수용 순서(오래된
    ///   순)를 유지하게 한다. 재파킹분은 신규 park 분보다 순번이 작으므로, 단일 호출·빈 큐 케이스에선
    ///   merge 결과가 옛 FRONT 삽입과 동일하다(동작 회귀 없음).
    /// ★왜 순서가 정확해야 하나(두 가지 손해)★: ① 수신자가 보는 배달 순서가 뒤집힌다(ADR-0104 "오래된 순
    ///   일괄" 은 큐 내부가 아니라 **수신자가 보는 순서**에 대한 약속) ② `handle_single_send` 의 FIFO 합류
    ///   판정이 큐 앞머리를 기준으로 하므로 나이 역전은 직발송 끼어들기로 번진다.
    /// ★전제(호출자 계약)★: `items` 는 **순번 오름차순**이어야 한다(drain 이 낸 순서 그대로거나 그 부분열 —
    ///   호출자가 인덱스 정렬로 보장한다). merge 는 두 오름차순 열을 합치는 것이라 이 전제가 깨지면 결과도
    ///   깨진다.
    /// ★`parked_at`·순번 모두 원래 값 유지★: 호출자가 원본 ParkedMessage 를 그대로 넘긴다 — TTL 이 연장되지
    ///   않고(오배송 방어) 수용 순서도 재부여되지 않는다(뒤로 밀리지 않는다).
    /// ★notice/message 무관 무조건 수용★: 재파킹은 kind 를 안 본다(이미 admitted). cap 회계는 신규 park 만.
    pub fn restore_ordered(&mut self, recipient: &str, items: Vec<ParkedMessage>) {
        if items.is_empty() {
            return;
        }
        let queue = self.queues.entry(recipient.to_string()).or_default();
        if queue.is_empty() {
            // 흔한 경로(락 안 복원 = drain 직후라 큐가 비어 있다) — merge 불요.
            queue.extend(items);
            return;
        }
        // 두 오름차순 열(재파킹분 · 큐 잔여)을 순번으로 merge. 동률은 구조적으로 없다(순번은 저장소가 유일
        //   부여자이고 강한 증가) — 그래도 방어적으로 재파킹분을 먼저 둔다(더 오래된 쪽).
        let existing = std::mem::take(queue);
        let mut merged = std::collections::VecDeque::with_capacity(existing.len() + items.len());
        let mut restored = items.into_iter().peekable();
        let mut remaining = existing.into_iter().peekable();
        loop {
            let take_restored = match (restored.peek(), remaining.peek()) {
                (Some(r), Some(q)) => r.admission_seq <= q.admission_seq,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if take_restored {
                merged.push_back(restored.next().expect("peek 직후"));
            } else {
                merged.push_back(remaining.next().expect("peek 직후"));
            }
        }
        *queue = merged;
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
    /// ★전량 스캔이다 — front 조기 종료 안 한다(round-4 finding 1 · load-bearing)★: 옛 구현은 "FIFO 니까
    ///   front 가 미만료면 뒤도 미만료" 로 첫 미만료에서 break 했다. 그 전제는 **큐 정렬축(admission 순번)과
    ///   만료축(`parked_at`)이 같다** 는 가정인데, 둘은 같지 않다: ① `park` 의 `parked_at` 은 **호출자가 주는
    ///   값**이라 저장소가 단조성을 보장할 수 없다 ② `park_pending` 은 락 획득 **전에** 시각을 떠서 경합 시
    ///   수용 순서와 시각이 역전될 수 있다. 그 경우 더 최근 항목이 앞에 서면 **뒤에 있는 만료 항목이 sweep
    ///   에서 영구히 가려진다**(TTL 이 무력화되고 장부에도 안 남는다 = 조용한 유실의 다른 얼굴). 그래서 전량
    ///   스캔으로 바꿨다 — 비용은 큐 길이 선형이고 규모가 극소해(수신자당 cap 100, 큐 수는 소수) 무의미하다.
    ///   `drain` 도 같은 이유로 전량 분할이다(그쪽은 원래부터).
    /// ★순수★: 만료 판정은 인자 `now` 로만 한다(모듈 헤더 불변식).
    pub fn sweep_expired(&mut self, now: Instant) -> Vec<ParkedMessage> {
        let mut expired = Vec::new();
        // 큐를 순회하며 만료분을 분리. 남는 항목은 원래 상대 순서(admission 순번 증가)를 그대로 유지한다.
        self.queues.retain(|_recipient, queue| {
            let scanned = std::mem::take(queue);
            for m in scanned {
                if m.is_expired(now) {
                    expired.push(m);
                } else {
                    queue.push_back(m);
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

    /// 수신자 큐의 현재 순서를 msg_id 로(앞→뒤, 관측·테스트용). 큐가 없으면 빈 목록.
    ///   순서 단언용 — 길이만으로는 재파킹의 나이 순서 역전이 안 잡힌다(round-4 finding 1).
    pub fn msg_ids(&self, recipient: &str) -> Vec<String> {
        self.queues
            .get(recipient)
            .map(|q| q.iter().map(|m| m.msg_id.clone()).collect())
            .unwrap_or_default()
    }

    /// ★이 id 를 힌트로 지목한 항목이 있는 큐 이름 목록(round-3 finding 2 — rename 고아 방지)★.
    ///
    /// ★왜 필요한가★: 파킹의 주소 축은 **이름**이라(service.rs `canonical_park_key`) busy 파킹은 **발송
    ///   시점의** canonical 이름 큐에 들어간다. 그런데 턴 종료 flush 는 그 에이전트의 **현재** 이름으로
    ///   진입하므로(tap 은 id 만 안다), 턴 중에 이름이 바뀌면(display_name 변경·cwd 파생 이름 변화) 옛 이름
    ///   큐를 아무도 열지 않아 그 배치가 TTL 까지 고아가 된다. 힌트로 역방향 조회를 하면 그 큐를 찾아낸다
    ///   (항목별 힌트 우선 해석은 flush 가 이미 하므로, 여기선 **어느 큐를 열어야 하나**만 답한다).
    /// ★비용(의도적 선택 — 인덱스 안 만든다)★: 전 큐 × 전 항목 선형 스캔이다. 규모가 극소하기 때문이다 —
    ///   큐는 파킹된 수신자 수(사람 대화 수준의 소수), 항목은 수신자당 cap 100. 별도 (id→큐) 인덱스를 두면
    ///   park/drain/restore_ordered/sweep 네 경로가 모두 인덱스를 정확히 갱신해야 하고, 한 곳만 놓쳐도 배달이
    ///   조용히 멈춘다(무손실 불변식과 정면 충돌). 유지 비용 대비 이득이 없어 스캔을 택했다.
    pub fn queues_with_hint(&self, id: AgentId) -> Vec<String> {
        self.queues
            .iter()
            .filter(|(_, q)| q.iter().any(|m| m.hinted_id == Some(id)))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// 저장소 전체가 비었나(전 수신자 큐 없음). 관측/테스트용.
    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }

    /// ★테스트 전용 손상 주입(C3 리뷰 fix 4)★ — 큐의 `idx` 번째 항목의 `envelope` 문자열을 임의 값으로
    ///   바꾼다. 파킹 payload 가 깨진 상황(형식 드리프트·메모리 손상)에서 **그 항목 하나만 열화되고 배치는
    ///   계속 나가는지**를 실제 flush 경로로 단언하기 위한 seam 이다. 운영 코드에서 부르지 않는다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn corrupt_envelope_for_test(&mut self, recipient: &str, idx: usize, envelope: String) {
        if let Some(q) = self.queues.get_mut(recipient) {
            if let Some(m) = q.get_mut(idx) {
                m.envelope = envelope;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 파킹 항목 생성 — id·kind·park 시각을 지정한다(id 힌트 없음 = 부재 파킹 모양).
    ///   `admission_seq` 는 `park` 이 덮어쓰므로 여기선 0(placeholder).
    fn parked(id: &str, kind: ParkKind, at: Instant) -> ParkedMessage {
        ParkedMessage {
            msg_id: id.to_string(),
            envelope: format!("<message>{id}</message>"),
            kind,
            parked_at: at,
            hinted_id: None,
            admission_seq: 0,
        }
    }

    #[test]
    fn hinted_id_survives_park_drain_and_restore_ordered() {
        // ★fix 2 회귀★: id 힌트는 저장소를 왕복(park→drain, 재파킹→drain)해도 보존돼야 한다 —
        //   힌트가 사라지면 exact-id 발송의 동명 blackhole 방어가 조용히 무력화된다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        let hint = AgentId::new_v4();
        let mut m = parked("m0", ParkKind::Message, t0);
        m.hinted_id = Some(hint);
        mb.park("alice", m).expect("park");
        let drained = mb.drain("alice", t0);
        assert_eq!(drained.deliverable[0].hinted_id, Some(hint), "drain 보존");
        mb.restore_ordered("alice", drained.deliverable);
        let again = mb.drain("alice", t0);
        assert_eq!(
            again.deliverable[0].hinted_id,
            Some(hint),
            "restore_ordered 왕복 후에도 보존"
        );
    }

    #[test]
    fn queues_with_hint_finds_only_queues_holding_that_id() {
        // ★round-3 finding 2★: 턴 중 이름이 바뀌면 옛 이름 큐를 열 단서는 id 힌트뿐이다 — 그 역방향 조회.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        let target = AgentId::new_v4();
        let other = AgentId::new_v4();
        let mut hinted = parked("m0", ParkKind::Message, t0);
        hinted.hinted_id = Some(target);
        mb.park("old-name", hinted).unwrap();
        let mut other_hint = parked("m1", ParkKind::Message, t0);
        other_hint.hinted_id = Some(other);
        mb.park("someone-else", other_hint).unwrap();
        // 힌트 없는 부재 파킹은 잡히지 않아야(그건 이름 규칙으로만 배달된다).
        mb.park("absent-name", parked("m2", ParkKind::Message, t0))
            .unwrap();

        assert_eq!(
            mb.queues_with_hint(target),
            vec!["old-name".to_string()],
            "그 id 를 힌트로 든 큐만"
        );
        assert!(
            mb.queues_with_hint(AgentId::new_v4()).is_empty(),
            "무관한 id 는 빈 목록"
        );
        // drain 으로 비면 더 이상 잡히지 않는다(빈 큐는 맵에서 제거).
        let _ = mb.drain("old-name", t0);
        assert!(mb.queues_with_hint(target).is_empty());
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
        // old 는 만료되게(t0), recent 는 살아 있게(t0 + 30m). now = t0 + TTL.
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
        // 오래된 것(t0) + 최근 것(t0 + 30m). now = t0 + TTL + 1ns 면 오래된 것만 만료.
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

    // ── restore_ordered(재파킹 무손실 복원 — finding 1 · round-4 finding 1) ─────────────
    #[test]
    fn restore_ordered_bypasses_cap_no_loss() {
        // ★조용한 유실 금지(ADR-0103 · finding 1)★: 큐가 이미 cap(100) 이면 park 는 반려하지만, restore_ordered
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
        // restore_ordered 는 cap 을 넘어서라도 되돌린다(유실 0).
        let older = vec![
            parked("old0", ParkKind::Message, t0),
            parked("old1", ParkKind::Message, t0),
        ];
        mb.restore_ordered("r", older);
        assert_eq!(
            mb.len("r"),
            MAILBOX_CAP + 2,
            "재파킹은 cap 을 우회해 admitted 항목을 되돌린다(유실 0)"
        );
    }

    #[test]
    fn restore_ordered_preserves_oldest_first_before_concurrent_parks() {
        // ★FIFO 역전 방지(finding 1)★: 재파킹분(더 오래됨)은 동시 park 된 신규분보다 앞서야 한다.
        //   시나리오: drain 으로 [old0,old1,old2] 를 꺼냈고 그 사이 new0·new1 이 park 됐다 → old0 배달 후
        //   실패 → [old1,old2] 재파킹. 이때 큐 = [new0,new1] 이므로 restore_ordered 후 = [old1,old2,new0,new1].
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
        mb.restore_ordered("r", older);
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
    fn restore_ordered_merges_two_batches_into_global_age_order() {
        // ★round-4 finding 1 회귀(핵심 버그)★: 한 flush 가 같은 이름 큐에 재파킹을 **두 번** 부르는 상황
        //   (동명 다수 = 타깃 2그룹이 각각 실패). 옛 FRONT 삽입은 두 번째 호출이 첫 번째보다 앞에 꽂혀
        //   [m1,m3,m0,m2] 로 나이 순서를 뒤집었다. merge 는 전역 수용 순서 [m0,m1,m2,m3] 를 유지한다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..4 {
            mb.park("dup", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        // 그룹 분할 = drain 이 낸 순서의 부분열(A = 짝수 인덱스, B = 홀수 인덱스).
        let drained = mb.drain("dup", t0).deliverable;
        let group_a: Vec<ParkedMessage> = drained.iter().step_by(2).cloned().collect();
        let group_b: Vec<ParkedMessage> = drained.iter().skip(1).step_by(2).cloned().collect();
        assert_eq!(
            group_a
                .iter()
                .map(|m| m.msg_id.as_str())
                .collect::<Vec<_>>(),
            vec!["m0", "m2"]
        );
        // 두 그룹이 각각(= 별개 호출로) 되돌아온다.
        mb.restore_ordered("dup", group_a);
        mb.restore_ordered("dup", group_b);
        assert_eq!(
            mb.msg_ids("dup"),
            vec!["m0", "m1", "m2", "m3"],
            "여러 번 복원해도 큐는 전역 수용 순서(오래된 순)를 유지해야"
        );
        // 이후 park 된 신규분은 항상 뒤에 붙는다(순번이 더 크다).
        mb.park("dup", parked("new", ParkKind::Message, t0))
            .unwrap();
        assert_eq!(
            mb.msg_ids("dup"),
            vec!["m0", "m1", "m2", "m3", "new"],
            "재파킹분이 신규분보다 앞"
        );
    }

    #[test]
    fn restore_ordered_interleaves_with_concurrently_parked_newer_items() {
        // 락 밖 복원 경로: 복원 대기 중 신규 park 가 끼어든 큐에 되돌려도 순번 순서가 지켜진다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..3 {
            mb.park("r", parked(&format!("old{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        let drained = mb.drain("r", t0).deliverable; // old0..old2 (순번 0..2)
        mb.park("r", parked("new0", ParkKind::Message, t0)).unwrap(); // 순번 3
        mb.restore_ordered("r", drained);
        assert_eq!(
            mb.msg_ids("r"),
            vec!["old0", "old1", "old2", "new0"],
            "동시 park 된 신규분은 재파킹분 뒤"
        );
    }

    #[test]
    fn sweep_surfaces_expired_hidden_behind_newer_front() {
        // ★round-4 finding 1 회귀(가려진 만료)★: 큐 정렬축(admission 순번)과 만료축(parked_at)은 다르다 —
        //   `park` 의 시각은 호출자가 주고(park_pending 은 락 밖에서 뜬다) 경합 시 역전될 수 있다. 옛 sweep 은
        //   front 가 미만료면 조기 종료해 **뒤에 있는 만료 항목을 영구히 가렸다**(TTL 무력화 + 장부 미기록).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // 수용 순서는 newer → older(역전) — front 는 미만료, 그 뒤가 만료.
        mb.park(
            "z",
            parked("newer", ParkKind::Message, t0 + Duration::from_secs(3600)),
        )
        .unwrap();
        mb.park("z", parked("older", ParkKind::Message, t0))
            .unwrap();
        let now = t0 + PARK_TTL;
        let expired = mb.sweep_expired(now);
        assert_eq!(
            expired
                .iter()
                .map(|m| m.msg_id.as_str())
                .collect::<Vec<_>>(),
            vec!["older"],
            "미만료 항목 뒤에 숨은 만료분도 sweep 이 걷어내야(전량 스캔)"
        );
        assert_eq!(
            mb.msg_ids("z"),
            vec!["newer"],
            "미만료분은 순서 그대로 잔존"
        );
    }

    #[test]
    fn restore_ordered_empty_is_noop() {
        let mut mb = Mailbox::new();
        mb.restore_ordered("r", Vec::new());
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
