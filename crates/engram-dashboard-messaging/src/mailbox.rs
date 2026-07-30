//! mailbox — 인메모리 파킹 저장소(spec §5 · ADR-0103 결정 5 · ADR-0104 idle 게이트 · **ADR-0111/0114**).
//!
//! ★역할★: 수신자별(이름 기반) FIFO 큐로 **아직 주입 못 한 메시지**를 잡아 둔다. 파킹 사유는 **셋**이다
//!   (ADR-0116 결정 1 로 잠듦이 부활 — ADR-0111 결정 1 의 "busy·주입 실패 전용" 은 폐기):
//!     ① busy 계열 — 수신자가 턴 진행 중이거나 선행 파킹분 뒤에 FIFO 로 합류. idle 진입 때 일괄 flush.
//!     ② 주입(write) 실패 — 그 순간 도달 불가. 다음 flush 계기(idle 진입·재등장)까지 대기.
//!     ③ **잠듦** — 프로세스는 닫혔지만 **프로필이 실재하는**(복원 가능한) 이름. flush 계기 = 재등장
//!        (스폰/복원/epoch — 기존 로스터 diff 기계 재사용).
//!   등장(스폰/epoch)·idle 진입 시 상위 서비스가 `drain` 으로 큐를 통째 비워 오래된 순으로 일괄 주입한다.
//!   ★**여전히 들어오지 않는 부류**(ADR-0116 결정 1 — 3분기)★: **어디에도 없는 이름**(로스터·프로필 둘 다에
//!   없다 — 오타·미스폰·삭제됨)은 `RECIPIENT_NOT_FOUND` 실패 행이고, **동명 다수**는 어느 층에서든
//!   `RECIPIENT_AMBIGUOUS` 실패 행이다(이름 키 파킹은 "먼저 복원된 쪽이 조용히 받는" 구멍이 된다).
//!   이 둘을 파킹으로 되돌리면 ADR-0116 위반이다.
//!   ★**턴 신호 없는 산 세션은 파킹 부류가 아니다 — 배달 부류다**(ADR-0116 결정 7)★: 게이트 없이 즉시
//!   주입되므로 busy 파킹이 없고, 그 부류가 이 큐에 들어오는 유일한 경로는 ②(주입 실패)다.
//!   (4차 초안의 `RECIPIENT_UNREACHABLE` 반려는 **폐기** — 되살리지 말 것.)
//!   ★삭제 정리★: 파킹 대기 중 그 이름의 프로필이 삭제되고 로스터에도 없으면 `purge_recipient` 로 전량
//!   종결한다(장부 `failed` + `RECIPIENT_DELETED` — 조용히 버리지 않는다, ADR-0116 결정 3).
//!
//! ★불변식★:
//!   - **FIFO(오래된 순)** — `drain`/`sweep_expired` 는 park 순서를 보존한다(ADR-0104 일괄·오래된 순 flush).
//!   - **큐 정렬축 = admission 순번(`ParkedMessage.admission_seq`)** — 큐는 앞→뒤로 순번이 **강한 증가**다
//!     (`park` 는 새 순번을 뒤에 붙이고, `restore_ordered` 는 순번 기준 merge 로 되꽂는다). "오래된 순" 의
//!     정본 축이 시계가 아니라 이 순번인 이유는 `admission_seq` 주석 참조(round-4 finding 1).
//!   - **용량 = 종류별 **독립 레인** 2개(round-6 재설계 — 예외 없는 단일 회계)**:
//!       ① `MAILBOX_CAP`(수신자당 message 100건) — message 를 전부 센다.
//!       ② `NOTICE_CAP`(수신자당 notice 64건) — message 백로그가 통지를 막지 않고, 그 역도 아니다.
//!     넘칠 때의 처리는 레인마다 다르다: **message** = **즉시 `MailboxFull` 반려**(부작용 0 — 상위가 그
//!     수신자만 실패 행으로 보고한다) · **notice** = **가장 오래된 notice** 를 회수하고 신규를 수용한다
//!     (**반려 없음** — 호출부가 결과를 버리므로 반려 = 조용한 유실). 회수분은 **항상 반환**한다(→ 상위가
//!     장부에 종점을 남긴다 — 어휘는 `ParkAdmitted` 주석).
//!     ★**압력 회수(message 레인의 잔해 은퇴)는 폐지됐다**(ADR-0114 결정 1 — ADR-0107 결정 2 개정)★:
//!     당시 근거는 "결박 폐지(ADR-0111 결정 6) + 없는 이름 반려(결정 1)로 '확실히 배달 불가한 파킹분' 이
//!     공집합" 이었다. ★그 논리 주장은 4차(ADR-0116)로 **약화됐다**★ — 복원되지 않는 잠든 이름의 파킹분은
//!     *복원 가능*이지 *복원됨*이 아니고 삭제 정리도 삭제 시점 단발이라 일부는 TTL 까지 남는다. **유계 근거는
//!     이제 논리 공집합이 아니라 TTL 24h + 레인 cap 이며**, 그 안에서 회수 폐지를 유지한다(spec §5. 회수
//!     재도입을 재론하려면 이 문장을 근거로 새 결정을 박을 것). message 레인의 유일한 정리 경로는 **TTL
//!     스윕**(24h 경과분, 60초 주기)이다.
//!   - **분모 = 큐 + in-flight(F1 — 구조적 유계의 핵심)** — flush 가 **락 밖으로 들고 나간 배치**도 그
//!     수신자 레인의 분모에 든다(`take_in_flight` → `settle_in_flight`). 큐만 세면 drain↔복원 사이의
//!     "큐가 비어 보이는 창" 에서 유입이 무제한 통과해, 되돌아온 배치와 합쳐져 사이클마다 cap 이 밀린다
//!     (실측 시나리오·불변식 증명은 `MAILBOX_CAP` 주석).
//!   - **TTL = 24h** — 초과 항목은 `sweep_expired` 가 걷어내 상위가 장부에 `expired` 로 남긴다(ADR-0105 —
//!     1h → 24h 상향, 인메모리 단계 한정).
//!   - **순수 + 주입 시계** — 만료 판정은 `park` 시각과 인자 `now` 의 차로만 한다(모듈 헤더 순수성 불변식).
//!   - **incarnation 결박(`bound_incarnation`)은 폐지됐다(ADR-0111 결정 6)** — 파킹분은 **같은 이름의 새
//!     화신(epoch)에게도 배달**된다(개인 편지와 동일 규칙). 그래서 이 저장소엔 결박 축도, 그 축을 보는
//!     가시성 필터(`visible_to`)도, 생사 스냅샷 인자도 없다. "그룹 방송은 발송 순간 화신에게만" 이 다시
//!     필요해지면 v2 개인 메일 옵션으로 무파괴 추가한다(spec §8) — 그룹 전용 규칙으로 되살리기 금지.
//!   - **FIFO 합류 = 큐 + in-flight(round-7)** — "먼저 나갈 게 있나" 의 정답은 큐만으로 안 나온다. flush 가
//!     락 밖으로 들고 나간 배치는 큐에 없어도 **그 수신자에게 먼저 도착할** 메시지라, 큐만 세면 직발송이
//!     진행 중인 배치를 앞지른다(수신자가 보는 순서 역전 — ADR-0104 계약 위반). 그래서 판정 동사는
//!     `has_pending_ahead`(= 큐 길이 + `in_flight_len`) 하나다.
// ADR-0103
// ADR-0104
// ADR-0105
// ADR-0111
// ADR-0114

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::PeerId;

/// TTL — 파킹 항목의 최대 생존 기간. 초과분은 `sweep_expired` 가 걷어낸다.
///
/// ★왜 24h(spec §5 정책 상수, ADR-0105 — 1h 에서 상향)★: 선례 조사(/research light, 2026-07-25) 상
///   업계 관행이 일 단위다(SQS 4일·Kafka 7일·Postfix 5일 — 1h 는 그 대비 이례적으로 짧았다). "살아있는
///   수신자는 TTL 면제"(liveness-aware) 는 조사한 6개 시스템(RabbitMQ·SQS·Kafka·Postfix·ejabberd·LLM
///   프레임워크) 어디에도 선례가 없어 채택하지 않는다 — 잠듦이든 busy 대기든 시계 기반 단일 규칙을 그대로
///   유지한다. 인메모리 단계 + cap 100(아래 `MAILBOX_CAP`) 이라 긴 TTL 의 비용은 ~0(데몬 재시작 시 전부
///   소멸) — **영속화(디스크) 단계가 오면 재설계 전제**(사용자 결정, 2026-07-25). 상위 서비스가 sweep
///   주기를 정한다(여기선 값만).
// ADR-0105
const PARK_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// ★message 레인 상한 — 수신자당 100건 **전량 계수**(round-6)★. 초과 시 **즉시 `MailboxFull` 반려**다
///   (ADR-0114 결정 1 — 압력 회수 폐지. 오래된 것 몰래 드롭 금지, spec §5 분기 3).
///
/// ★왜 100건(spec §5 정책 상수)★: 폭주하는 발신자가 한 수신자의 메일박스를 무한히 부풀려 메모리를 잠식하는
///   것을 막는 방어선이다. 초과를 조용히 drop-head 하면 유실이 은폐되므로(ADR-0103 거부 대안), 대신 신규를
///   즉시 반려해 발신자에게 가시화한다(다중 수신자에서도 **그 수신자만** 실패 행 — 전체 반려 승격 없음).
/// ★왜 예외 없는 전량 계수인가(round-6 — 예외가 구멍을 만들었다)★: 옛 구현은 분모에서 "지금 이 incarnation
///   에게 배달 불가한(= 결박이 다른) 항목" 을 뺐다. 그러면 같은 이름을 새 PeerId 로 갈아치울 때마다 분모가
///   0 으로 리셋돼 **incarnation 당 100건씩** 새로 수용됐고(TTL 24h 창 안에서 사실상 무계), 그걸 막으려고
///   물리 상한이라는 **두 번째 회계**를 얹자 그 상한을 우회하는 경로(`restore_ordered`)와 두 상한 모두를
///   면제받는 종류(notice)가 각각 새 구멍이 됐다. 그래서 분모에서 예외를 전부 걷어냈다.
/// ★그 예외를 떠받치던 "압력 회수" 도 이제 없다(ADR-0114 결정 1)★: 결박 폐지로 "배달 불가 잔해" 자체가
///   생기지 않으므로 회수 대상이 공집합이다 — 큐가 100건이면 그건 **진짜 배달될 메일**이 밀린 것이고,
///   정답은 잔해 사냥이 아니라 가시적 반려다. 정리는 TTL 스윕 하나가 맡는다.
/// ★분모는 큐가 아니라 **큐 + in-flight** 다(F1 — round-6 의 유계 주장이 틀렸던 지점)★: round-6 은 "큐
///   길이 ≤ cap 이니 총량이 유계" 라고 적었는데, `restore_ordered`(cap 우회 무손실 복원)가 그 논증을
///   **사이클 간에** 무너뜨렸다. 실측 인터리빙: ① flush 가 락 안에서 큐를 통째로 비운다 ② 락을 놓고
///   inject 하는 동안 큐는 **비어 있으므로** 동시 발송 k 건이 cap 검사를 무사통과해 들어온다 ③ inject 가
///   실패해 배치 N 건이 되돌아온다 → 큐 = N + k. 다음 사이클의 배치는 N + k 라 매 사이클 **+k** 로 자란다
///   (옛 주석의 "초과폭은 한 배치 이하" 는 그 한 배치 자체가 이미 초과분을 품고 있어 순환 논증이었다).
///   그래서 분모에 **in-flight**(= flush 가 락 밖으로 들고 나가 아직 정산되지 않은 그 수신자 몫)를 더한다.
/// ★cap 이 묶는 것은 **보관된 메일**이다 — 동시 즉시 주입은 아니다(리뷰 blind #4 — 명시)★: 큐에 들어가지
///   않고 곧바로 수신자 stdin 으로 나간 메시지는 저장되지 않으므로 이 분모에 들어오지 않는다. 그래서 idle
///   수신자에게 동시 발송 101건이 몰리면 101건이 그대로 주입될 수 있고, **그건 cap 위반이 아니다**(메일박스
///   메모리는 여전히 유계다). 이 상수의 목적은 "쌓이는 메일의 상한" 이지 "초당 주입 수 제한" 이 아니다 —
///   후자가 필요해지면 그건 rate limit 이라는 다른 장치다.
///   ★불변식(정확한 등식 — 여유폭 0)★: `queue_lane + in_flight_lane ≤ cap`. 근거는 각 동사가 이 합을
///   보존하거나 줄이기 때문이다 — `take_in_flight` 는 큐에서 뺀 만큼 in-flight 를 올리고(합 불변),
///   `restore_ordered` + 같은 락 구간의 정산은 in-flight 에서 뺀 만큼 큐에 되돌리며(합 불변), 배달 완료
///   정산은 합을 줄이고, `park` 은 합이 cap 을 넘기면 반려한다. notice 레인만 예외적으로
///   **+1 의 일시 초과**가 가능하다(그 근거는 `NOTICE_CAP` 주석).
const MAILBOX_CAP: usize = 100; // ADR-0107

/// ★notice 레인 상한 — 수신자당 64건(round-6 도입, **ADR-0114 결정 2 로 20 → 64 상향**)★. `MAILBOX_CAP`
///   과 **완전히 독립**이다: notice 는 message 분모에, message 는 notice 분모에 들어가지 않는다. 초과 시
///   **가장 오래된 notice 를 회수**하고 신규를 수용한다(반려 없음 — `park` 참조).
///
/// ★왜 별도 레인인가(옛 "cap 예외" 를 대체)★: 회신 계약의 기한 초과 통지는 가득 찬 메일박스에 막히면
///   계약이 반쪽 난다(ADR-0103 불변식). 옛 구현은 그걸 "notice 는 cap 을 세지 않는다" 는 **면제**로 풀었는데,
///   면제는 곧 무계였다 — 근거로 든 "오픈 계약 수 상한(`ledger::MAX_OPEN_REQUESTS`)이 notice 수를 묶는다"
///   는 **거짓**이다: `due_timeouts` 는 `notified` 를 즉시 세워 그 계약 자리를 비우므로, 앞선 통지가 큐에
///   그대로 파킹돼 있는 채로 다음 물결이 계약을 새로 열 수 있다(수신자가 오래 잠들어 있으면 무한히 반복).
///   레인을 나누면 두 요구가 동시에 성립한다 — message 백로그가 통지를 막지 못하고(면제의 목적), 통지도
///   무계가 아니다(면제의 대가를 제거).
/// ★왜 64건인가(정책 상수 — 조율 가능, ADR-0114 결정 2)★: notice 는 **데몬이 만드는 인프라 통지**라
///   정상 운영에서 큐에 여러 건이 쌓일 이유가 없었지만, **다중 수신자 request** 가 열리면서(ADR-0111 결정 5)
///   한 발송이 계약 N개를 열고 기한 초과 통지도 **계약별 1건**씩 난다 — 20건은 그 폭에 비해 좁다(통지 병합
///   시스템은 만들지 않는다는 결정과 짝). 상한을 올려 유실 창 자체를 줄인다. 그래도 넘치면 **가장 오래된
///   통지**는 이미 가치가 낮으므로(최신 통지들이 같은 종류의 사실을 더 정확한 시점으로 전달한다) 신규를
///   반려하는 대신 그것을 회수한다(회수분은 장부에 `skipped` 로 남아 감사 가능 — `ParkAdmitted`).
/// ★이 레인만 **+1 의 일시 초과**가 가능하다(F1 — 명시 계약)★: 회수 대상은 **큐에 있는** notice 뿐인데
///   분모는 큐 + in-flight 라, 통지 64건이 전부 flush 로 나가 있는 순간의 신규 통지는 회수할 대상을 큐에서
///   못 찾고도 **수용된다**(반려 갈래가 없으므로). 그 순간 `queue + in_flight = NOTICE_CAP + 1` 이다. 그
///   상태는 스스로 되돌아온다 — 다음 통지 park 이 초과분 전체를 `want` 로 계산해 걷어낸다. message 레인은
///   반려 갈래가 있어 이 초과가 아예 없다(합이 정확히 cap 에서 멈춘다).
// ADR-0114 (NOTICE_CAP 20 → 64)
const NOTICE_CAP: usize = 64; // ADR-0107

/// 파킹 항목의 종류 — **용량 레인**을 가르는 축(round-6). 각 종류는 자기 레인의 상한만 본다(상대 레인의
///   길이는 서로에게 보이지 않는다 — 모듈 헤더 "독립 레인").
///
/// ★왜 종류로 레인을 가르나★: `<notice>`(데몬 전용 인프라 통지, 특히 request 타임아웃)는 메일박스가 가득
///   차도 발신자에게 도달해야 회신 계약이 성립한다(ADR-0103 불변식). 반대로 통지가 밀려 들어와 동료 메일을
///   밀어내도 안 된다. 두 방향을 동시에 만족시키는 최소 구조가 **독립 상한 2개**다(옛 "notice 면제" 가
///   왜 폐기됐는지는 `NOTICE_CAP` 주석).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkKind {
    /// 동료 발신(`<message>`) — `MAILBOX_CAP` 레인.
    Message,
    /// 데몬 통지(`<notice>`) — `NOTICE_CAP` 레인(반려되지 않는다 — 넘치면 가장 오래된 통지가 회수된다).
    Notice,
}

/// 파킹된 메시지 1건. 봉투 텍스트(주입 시 그대로 stdin 에 밀어넣을 완성 문자열)와 회계용 메타를 담는다.
///
/// ★envelope = 완성된 봉투 문자열★: 이 모듈은 순수 저장소라 봉투를 **조립하지 않는다** — 상위(service 가
///   부르는 `envelope.rs` wrap point)가 만든 완성 문자열을 그대로 보관·반환한다(단일 wrap point 불변식, ADR-0096). 여기선 그게
///   어떤 포맷인지 모른다(불투명 텍스트).
/// ★parked_at★: TTL 판정 기준 시각. 상위가 park 호출 시점의 `now` 를 주입한다(순수성 불변식).
#[derive(Debug, Clone)]
pub struct ParkedMessage {
    /// ledger 상관용 논리 메시지 id(상위가 부여). 저장소는 값으로만 나른다.
    pub msg_id: String,
    /// 완성된 봉투 문자열(주입 시 그대로 stdin 에 밀어넣음). 저장소는 조립하지 않는다(불투명).
    pub envelope: String,
    /// 항목 종류 — **용량 레인**을 고르는 축(`ParkKind` 주석: message = `MAILBOX_CAP` · notice = `NOTICE_CAP`).
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
    ///   수신자를 이미 해석했다면** 그 PeerId. busy 대기·주입 실패 파킹은 **항상** 값이 있고, 데몬
    ///   통지(notice)는 요청 발신자 id 를 싣는다.
    /// ★**잠듦 파킹은 `None` 이다**(4차 — ADR-0116 결정 1)★: 그 순간 산 incarnation 이 없어 힌트로 실을 id 가
    ///   존재하지 않는다. 그래서 그 항목은 **이름 큐로만** 열리고, 복원된 세션의 canonical 이름이 파킹 키와
    ///   같아야 배달된다 — 잠듦 이름을 산 세션과 **같은 규칙으로** 파생하는 것이 load-bearing 인 이유다
    ///   (spec §5 "프로필 쪽 이름 = 산 세션과 동일한 파생 규칙"). 3차 서술("None 은 테스트 seam 잔여뿐")은 폐기.
    ///
    /// ★왜 필요한가(이름-키 파킹의 사각지대)★: 파킹의 주소 단위는 **이름**이다(respawn 생존 —
    ///   근거는 `ResolvedRecipient.key` 주석). 그런데 flush 는 "그 이름의 도달 후보가 **정확히 1개**" 일 때만 배달한다(동명
    ///   다수는 보류). 여기서 구멍이 생긴다: exact-PeerId 로 지목한 발송은 동명 모호성을 **의도적으로
    ///   통과**하는데(id 가 명시적 승자), 그 수신자가 turn 중이라 이름-키로 park 되면 동명이 둘인 동안
    ///   flush 가 영영 보류돼 TTL 까지 blackhole 이 된다. 그래서 park 시점에 해석된 id 를 힌트로 함께
    ///   보관해, flush 가 **그 id 가 아직 살아 있으면 이름 유일성과 무관하게** 그쪽으로 배달한다.
    /// ★힌트는 권위가 아니라 우선순위다★: 그 id 가 죽었으면(재스폰 등) 무시하고 이름 규칙으로 되돌아간다
    ///   — 그래서 "재스폰된 동명이 파킹을 이어받는다" 는 이름-키 설계가 그대로 유지된다.
    pub hinted_id: Option<PeerId>,
}

impl ParkedMessage {
    /// `now` 기준으로 TTL 에 도달했나. 경계(정확히 TTL)는 **만료**(`>=` 비교 — 아래 테스트 고정).
    ///
    /// ★경계 규약(load-bearing)★: `elapsed >= PARK_TTL` 이라 정확히 TTL 이 지난 순간부터 만료다(경계 포함).
    ///   `>` 가 아니라 `>=` 를 쓰는 이유는 "TTL = 최대 생존 기간" 이라는 상한 의미와 정합하기 위함이다 —
    ///   TTL 을 꽉 채운 항목은 더 살려 둘 이유가 없다(경계에서 즉시 만료가 상한 의미에 부합). 이 경계는 단위
    ///   테스트(`ttl_boundary_*`)가 고정한다 — 바꾸면 회귀.
    /// ★`pub(crate)` 인 이유(F3)★: 압력 회수분의 장부 어휘를 상위(`service::park_into`)가 가를 때 이 판정을
    ///   **그대로 재사용**해야 한다(TTL 지난 회수분 = `expired`, 아니면 `skipped`). 상위가 24h 리터럴을
    ///   복제하면 두 곳이 갈릴 수 있으므로, 상수도 비교 부호도 여기 하나로 묶어 두고 함수만 연다.
    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.parked_at) >= PARK_TTL
    }
}

/// park 반려 사유 — 상위가 wire 에러 코드로 매핑한다(현재 유일: cap 초과 → `MAILBOX_FULL`, spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkError {
    /// message 레인(큐 + in-flight)이 `MAILBOX_CAP` 에 도달 → `MAILBOX_FULL`(ADR-0114 결정 1: 회수 시도
    ///   없이 즉시 반려). notice 레인은 이 값을 절대 내지 않는다(넘치면 회수하고 수용 — `park` 참조).
    MailboxFull,
}

/// `park` 수용 결과 — 그 park 이 자리를 만드느라 **걷어낸 항목**을 함께 돌려준다(round-6).
///
/// ★왜 반환값인가(`DrainOutcome`/`ExpiredParked` 와 같은 패턴)★: 저장소는 순수라 장부를 모른다. 걷어낸
///   항목을 여기서 조용히 떨구면 그건 **은폐된 유실**이다(장부엔 `pending` 인데 큐엔 없음 = 유령 pending).
///   그래서 "큐에서 뺐다" 는 사실을 항목째로 호출자에게 넘겨, 상위가 장부에 `skipped` 를 남기고 락 밖에서
///   로깅하게 한다 — 경로가 아니라 **반환값**으로 유실을 막는 이 모듈의 기존 규율 그대로다.
/// ★오늘 이 채널을 쓰는 건 **notice 레인 하나뿐**이다(ADR-0114 결정 1)★: message 레인의 압력 회수가
///   폐지돼(모듈 헤더) 남은 회수 사유는 "가장 오래된 통지가 더 최신 통지에 밀림" 뿐이다. 그래도 값 채널을
///   유지하는 이유는 조용한 유실 금지 규율(회수 사실은 반드시 장부 종점으로 남는다)이 그대로이기 때문이다.
/// ★어휘는 **기본 `skipped`, 단 TTL 이 우선한다**(F3)★: 밀려난 옛 통지는 `expired`(시계 사실)가 아니라
///   `skipped`("그 수신자에게는 배달하지 않음")가 정확한 사실이다. **그러나** sweep 주기(60s)와 TTL(24h)
///   사이의 틈 때문에 "이미 TTL 을 넘겼지만 아직 안 걷힌" 항목이 회수될 수 있고, 그건 spec §5 계약상
///   `expired` 다(시계가 먼저 그 항목의 운명을 정했다). 그래서 **판정은 저장소가 하지 않는다** — 회수분을
///   항목째 돌려주고, 상위가 `ParkedMessage::is_expired(now)` 로 둘을 갈라 전이한다(`service::park_into`).
#[derive(Debug, Default)]
pub struct ParkAdmitted {
    /// 이 park 이 자리를 만드느라 걷어낸 항목(**오래된 순**). 보통 비어 있다.
    pub retired: Vec<ParkedMessage>,
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

/// `sweep_expired` 가 걷어낸 만료 항목 1건 — **어느 수신자 큐에 있었는지**를 항목과 함께 나른다(C4).
///
/// ★왜 recipient 를 함께 돌려주나(load-bearing — 1 msg_id : N 배달기록, spec §4 · ADR-0104)★: 장부 레코드의
///   키는 `(msg_id, recipient)` 다. 그룹 방송은 한 msg_id 에 수신자별 레코드가 **N개** 있으므로, 만료된
///   항목이 어느 레코드인지 msg_id 만으로는 특정할 수 없다 — 상위가 msg_id 로 "첫 pending 레코드" 를 역조회
///   하면 **엉뚱한 멤버의 레코드**를 expired 로 전이할 수 있다(C1 시절 헬퍼의 위험, 그 앵커가 지목한 결함).
///   저장소는 항목이 어느 큐에 있었는지 아는 유일한 지점이라, 여기서 그 사실을 함께 실어 보낸다.
// ADR-0104
#[derive(Debug, Clone)]
pub struct ExpiredParked {
    /// 이 항목이 있던 수신자 큐 키(= 장부 레코드의 `to`). 상위가 `(msg_id, recipient)` 로 정확히 전이한다.
    pub recipient: String,
    /// 만료된 파킹 항목.
    pub msg: ParkedMessage,
}

/// ★in-flight 영수증(F1 · load-bearing)★ — flush 가 락 밖으로 들고 나간 배치의 **레인별 건수**.
///
/// ★왜 값 타입 티켓인가★: drain 과 복원 사이에 messaging 락이 **풀린다**(inject 는 락 밖 계약). 그래서
///   "지금 몇 건이 나가 있나" 를 스택 가드로 들 수 없고 저장소가 들어야 하는데, 저장소는 누가 얼마를
///   가져갔는지 스스로 알 수 없다. 그래서 가져간 쪽에 영수증을 발급하고(`take_in_flight`) 반드시 반납받는다
///   (`settle_in_flight`). 반납이 누락되면 그 레인의 분모가 영구히 부풀어 유입이 막히므로, 상위는 이 값을
///   **단일 출구 가드**로 들고 다닌다(service.rs `FlightSettle`).
/// ★부분 반납이 가능하다★: 배치 중 배달된 항목·되돌린 항목을 그때그때 떼어 반납한다(`split`) — 그래야
///   분모가 실제 미결 건수와 같아진다. 이중 반납은 `split` 이 잔량을 넘겨 떼지 않으므로 구조적으로 없다.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlightTicket {
    message: usize,
    notice: usize,
}

impl FlightTicket {
    /// 반납할 게 남았나(대부분의 경로에서 곧바로 0 — 락 획득 자체를 아끼는 데 쓴다).
    pub fn is_zero(&self) -> bool {
        self.message == 0 && self.notice == 0
    }

    fn lane_mut(&mut self, kind: ParkKind) -> &mut usize {
        match kind {
            ParkKind::Message => &mut self.message,
            ParkKind::Notice => &mut self.notice,
        }
    }

    fn lane(&self, kind: ParkKind) -> usize {
        match kind {
            ParkKind::Message => self.message,
            ParkKind::Notice => self.notice,
        }
    }

    fn add(&mut self, kind: ParkKind) {
        *self.lane_mut(kind) += 1;
    }

    /// 이 영수증에서 `kinds` 만큼을 떼어 **부분 반납용 영수증**을 만든다. ★잔량을 넘겨 떼지 않는다★ —
    ///   그게 이중 반납(분모가 실제보다 작아져 cap 이 뚫리는 사고)을 막는 유일한 장치다.
    pub fn split(&mut self, kinds: impl IntoIterator<Item = ParkKind>) -> FlightTicket {
        let mut out = FlightTicket::default();
        for kind in kinds {
            if self.lane(kind) > 0 {
                *self.lane_mut(kind) -= 1;
                out.add(kind);
            }
        }
        out
    }
}

/// 수신자 이름(String, WYSIWYA — ADR-0101) → FIFO 큐 저장소. **파킹 사유 셋 공용**(busy 계열·주입 실패·
/// 잠듦 — spec §5 분기 3, 모듈 헤더가 정본. 4차로 잠듦이 부활했다: ADR-0116 결정 1).
///
/// ★순수★: 시간 의존 메서드는 전부 `now: Instant` 를 주입받는다(모듈 헤더 불변식). 내부에 시계 없음.
#[derive(Debug, Default)]
pub struct Mailbox {
    /// 수신자 이름별 FIFO 큐. `VecDeque` 는 push_back(park)·drain(앞에서부터) 모두 오래된 순 보존.
    queues: HashMap<String, std::collections::VecDeque<ParkedMessage>>,
    /// ★수신자별 in-flight 건수(F1)★ — 큐에서 나갔지만 아직 종점(배달/복원)이 확정되지 않은 몫. cap 분모는
    ///   `queue + 이 값`이다(모듈 헤더 "분모 = 큐 + in-flight"). 0 이 되면 항목을 지운다(빈 키 누적 방지).
    in_flight: HashMap<String, FlightTicket>,
    /// 다음 admission 순번(전 수신자 공유 단조 카운터 — `ParkedMessage.admission_seq` 부여자).
    ///   수신자별이 아니라 저장소 전역인 이유: 한 이름 큐에 여러 타깃 몫이 섞여도(동명 다수) 순번 하나로
    ///   전역 수용 순서를 표현할 수 있고, u64 라 실질적으로 소진되지 않는다.
    next_seq: u64,
}

/// ★notice 레인 압력 회수 — 가장 오래된 notice 를 최대 `want` 개 걷어낸다(round-6 · ADR-0114 로 축소)★.
///
/// - 대상은 **notice 뿐**이다(message 레인의 압력 회수는 폐지 — 모듈 헤더 · ADR-0114 결정 1). 술어에
///   걸리지 않는 항목은 **절대** 건드리지 않는다(회수가 산 메일을 잡아먹지 않는다는 보장).
/// - **오래된 순의 근거 = 큐 정렬축**: 큐는 admission 순번 **강한 증가**라(모듈 헤더 불변식) 앞→뒤 순회가
///   곧 오래된 순이다. 별도 정렬을 하지 않는 이유이자, 그 불변식이 깨지면 여기 순서도 깨진다는 뜻이다.
/// - 남는 항목의 상대 순서는 그대로 보존한다(회수는 순서를 재배열하지 않는다 — FIFO 불변식).
// ADR-0114 (message 레인 회수 폐지 — notice 전용으로 축소)
fn retire_oldest(
    queue: &mut std::collections::VecDeque<ParkedMessage>,
    want: usize,
    mut evictable: impl FnMut(&ParkedMessage) -> bool,
) -> Vec<ParkedMessage> {
    let mut retired = Vec::with_capacity(want);
    let mut kept = std::collections::VecDeque::with_capacity(queue.len());
    for m in std::mem::take(queue) {
        if retired.len() < want && evictable(&m) {
            retired.push(m);
        } else {
            kept.push_back(m);
        }
    }
    *queue = kept;
    retired
}

impl Mailbox {
    /// 빈 메일박스.
    pub fn new() -> Self {
        Self::default()
    }

    /// 메시지를 수신자 큐 끝에 park(FIFO). message 레인이 가득 차면 **즉시 `MailboxFull` 반려**하고,
    ///   notice 레인은 가장 오래된 통지를 회수하고 신규를 수용한다(notice 는 절대 반려하지 않는다).
    ///
    /// ★용량 = 종류별 독립 레인, 예외 없음(round-6 재설계 · F1 분모 보정 · ADR-0114 회수 폐지)★:
    ///   - **분모(두 레인 공통)** = 큐의 그 레인 항목 수 **+ 그 수신자 레인의 in-flight 건수**(= flush 가
    ///     락 밖으로 들고 나가 아직 정산되지 않은 몫 — `take_in_flight`). 큐만 세면 drain↔복원 사이에
    ///     "큐가 비어 보이는 창" 이 열려 cap 이 사이클마다 밀린다(F1 — `MAILBOX_CAP` 주석의 실측 시나리오).
    ///   - **message**: 전량 계수. `MAILBOX_CAP` 이상이면 **회수 시도 없이 즉시** `MailboxFull` 반려다
    ///     (ADR-0114 결정 1 — 회수 장치 폐지. 유계 근거는 TTL 24h + 이 cap 이다, 모듈 헤더 4차 보정).
    ///     상위는 그 수신자만 실패 행으로 보고한다(다중
    ///     수신자에서 전체 반려로 승격하지 않는다 — spec §5).
    ///   - **notice**: message 는 분모에 세지 않는다. `NOTICE_CAP` 이상이면 **가장 오래된 notice** 를 회수하고
    ///     신규를 수용한다 — **반려 갈래가 없다**. 왜: `deliver_notice` 는 park 결과를 버리므로 여기서
    ///     반려하면 통지가 **조용히 사라진다**(회신 계약이 반쪽 — ADR-0103 불변식). 그 대가로 회수 대상이
    ///     큐에 모자랄 때 +1 의 일시 초과가 난다(`NOTICE_CAP` 주석). 회수분은 `ParkAdmitted.retired` 로
    ///     **돌려준다**(저장소는 순수 — 장부 종점을 찍는 건 상위 몫, 조용한 유실 금지).
    /// ★반려 경로는 부작용 0★: 반려면 큐를 건드리지도 순번을 태우지도 않는다 — "반려면 저장 자체를 안
    ///   한다"(장부도 안 남는다)는 상위 계약과 정합.
    /// ★notice 의 `want` 가 1보다 클 수 있다★: `restore_ordered`(재파킹)는 cap 을 우회하므로 큐가 일시적으로
    ///   상한을 넘을 수 있다. 그래서 "1자리 확보" 가 아니라 **초과분 전체**를 계산해 한 번에 정리한다.
    // ADR-0107 (레인별 회계 — 반려는 부작용 0)
    // ADR-0114 (message 레인 압력 회수 폐지 — 즉시 반려)
    pub fn park(
        &mut self,
        recipient: &str,
        mut msg: ParkedMessage,
    ) -> Result<ParkAdmitted, ParkError> {
        let kind = msg.kind;
        // ★분모에 in-flight 를 더한다(F1)★ — 큐에서 나가 있는 그 수신자 몫도 아직 "이 수신자 앞 미결
        //   메시지" 다. 필드가 서로 달라 queues 의 가변 대여와 공존한다(disjoint field borrow).
        let in_flight = self
            .in_flight
            .get(recipient)
            .map(|t| t.lane(kind))
            .unwrap_or(0);
        let queue = self.queues.entry(recipient.to_string()).or_default();
        let lane_len = queue.iter().filter(|m| m.kind == kind).count() + in_flight;
        let cap = match kind {
            ParkKind::Message => MAILBOX_CAP,
            ParkKind::Notice => NOTICE_CAP,
        };
        // 이 항목까지 넣었을 때의 초과분(0 이면 여유 있음 = 대부분의 경로).
        let want = (lane_len + 1).saturating_sub(cap);
        let mut retired = Vec::new();
        if want > 0 {
            match kind {
                // message: 회수 없음 — 즉시 반려(부작용 0). ADR-0114 결정 1.
                ParkKind::Message => return Err(ParkError::MailboxFull),
                // notice: 반려 없음 — 가장 오래된 통지를 회수한다. ★`want` 가 안 채워질 수 있다(F1)★:
                //   분모에 in-flight 가 들어가므로 "레인이 찼는데 큐엔 회수할 통지가 없는" 순간이 존재한다.
                //   그때도 신규는 수용한다(반려 = 조용한 유실) — 그 대가가 +1 일시 초과다(NOTICE_CAP 주석).
                ParkKind::Notice => {
                    retired = retire_oldest(queue, want, |m| m.kind == ParkKind::Notice);
                }
            }
        }
        // admission 순번은 **수용이 확정된 뒤** 저장소가 부여한다(반려분은 번호를 태우지 않는다). 호출자 값은
        //   덮어쓴다 — 부여자가 여기 하나여야 큐의 "순번 강한 증가" 불변식이 성립한다.
        msg.admission_seq = self.next_seq;
        self.next_seq += 1;
        queue.push_back(msg);
        Ok(ParkAdmitted { retired })
    }

    /// ★신규 park 이 **지금 수용될 수 있나**(순수 조회 — ADR-0114 이후 가능해진 예측)★.
    ///
    /// ★왜 필요한가(load-bearing — 봉투 `to` 동결의 전제)★: 다중 수신자 발송은 **전 수신자의 수용 판정이
    ///   끝난 뒤에야** 봉투 `to` 값을 확정할 수 있는데(spec §1 — 실패 행은 세지 않는다), 파킹은 그 값을
    ///   payload 에 실어야 하므로 "판정 → to 확정 → 파킹" 순서가 강제된다. 즉 파킹 **전에** cap 결과를
    ///   알아야 한다. 압력 회수가 있던 시절엔 이 예측이 불가능했지만(회수 여부가 park 안에서 정해졌다),
    ///   회수가 폐지되면서 판정이 순수 산술이 됐다 — 그래서 예측과 실제가 **절대 갈리지 않는다**(같은 락
    ///   구간 안에서 부르는 한).
    /// ★notice 는 항상 true★: 그 레인은 반려 갈래가 없다(`park` 참조).
    pub fn can_admit(&self, recipient: &str, kind: ParkKind) -> bool {
        let cap = match kind {
            ParkKind::Message => MAILBOX_CAP,
            ParkKind::Notice => return true,
        };
        let in_flight = self
            .in_flight
            .get(recipient)
            .map(|t| t.lane(kind))
            .unwrap_or(0);
        let queued = self
            .queues
            .get(recipient)
            .map(|q| q.iter().filter(|m| m.kind == kind).count())
            .unwrap_or(0);
        queued + in_flight < cap
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
    /// ★유계의 근거는 "한 배치 이하" 가 **아니라** in-flight 회계다(F1 — 옛 논증 폐기)★: round-6 은 여기에
    ///   "초과폭은 한 배치 이하다 — drain 은 큐를 통째로 비우므로 cap 안이던 옛 큐보다 커질 수 없다" 고
    ///   적었는데, **그 옛 큐 자체가 이미 초과분을 품고 있을 수 있어** 사이클 간에 순환 논증이었다(실측
    ///   인터리빙은 `MAILBOX_CAP` 주석). 지금은 drain 한 배치가 **분모에 계속 잡혀 있으므로**(`take_in_flight`)
    ///   drain↔복원 창에서 들어올 수 있는 신규 유입 k 는 `cap − (큐 + in-flight)` 로 이미 제한돼 있고,
    ///   복원은 in-flight 에서 큐로 **자리를 옮길 뿐**이라(같은 락 구간에서 그만큼 정산한다 — service.rs
    ///   `flush_for` 실패 갈래) 합이 늘지 않는다. 즉 이 우회는 cap 을 뚫지 못한다 — `queue + in_flight ≤ cap`
    ///   이 그대로 성립한다. **cap 을 세지 않는다는 성질 자체는 유지**된다(무손실 복원이 목적이고, 유입
    ///   제한은 park 쪽에서 이미 걸렸다).
    ///   ★되돌아오는 배치를 줄이는 것도 그대로다★: flush 는 로스터를 손에 쥔 유일한 지점이라, 복원 **전에**
    ///   확정 사망 결박분(같은 PeerId 의 더 높은 epoch)을 걷어내고 장부에 `skipped` 를 찍는다(service.rs
    ///   `flush_for`) — 즉 되돌아오는 배치에는 증거 있는 사망분이 이미 빠져 있다.
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

    /// ★락 밖으로 나가는 배치를 in-flight 로 등록한다(F1 · load-bearing)★ — flush 가 `drain` 결과 중 **실제로
    ///   락을 떠나 주입 대상이 되는 몫**만 여기에 신고하고, 반환 영수증을 반드시 `settle_in_flight` 로 반납한다.
    ///
    /// ★왜 `drain` 이 자동으로 안 하고 별도 동사인가(의도적)★: drain 이 낸 항목 전부가 락을 떠나는 게
    ///   아니다 — busy 타깃 몫·배달 경로 없는 몫은 **같은 락 구간에서** 곧바로 되돌아가고(`restore_ordered`),
    ///   확정 사망분·만료분은 그 자리에서 장부 종점을 찍고 사라진다. 그 몫까지 in-flight 로 잡으면 분모가
    ///   근거 없이 부풀어(= 락 안에서만 존재한 창을 분모에 반영) 정상 유입이 `MailboxFull` 로 반려된다.
    ///   구멍은 **락이 풀리는 구간**에만 있으므로, 등록 단위도 정확히 그 구간으로 맞춘다.
    /// ★호출자 계약★: 등록한 건수는 배달 완료·복원·포기 중 무엇이 되든 **전부** 반납돼야 한다. 반납이
    ///   누락되면 그 레인의 분모가 영구히 부풀어 유입이 막히므로, 상위는 단일 출구 가드로 들고 다닌다
    ///   (service.rs `FlightSettle` — early break·언와인딩까지 덮는다).
    // ADR-0107 (in-flight 회계 — 구조적 유계 등식)
    pub fn take_in_flight<'m>(
        &mut self,
        recipient: &str,
        batch: impl IntoIterator<Item = &'m ParkedMessage>,
    ) -> FlightTicket {
        let mut ticket = FlightTicket::default();
        for m in batch {
            ticket.add(m.kind);
        }
        if !ticket.is_zero() {
            let slot = self.in_flight.entry(recipient.to_string()).or_default();
            slot.message += ticket.message;
            slot.notice += ticket.notice;
        }
        ticket
    }

    /// ★in-flight 영수증 반납(F1)★ — `take_in_flight` 로 올려 둔 분모를 그만큼 내린다. 0 이 되면 키를 지운다.
    ///
    /// ★saturating 인 이유★: 여기서 패닉하면 언와인딩 중 이중 패닉으로 프로세스가 죽는다. 과다 반납은
    ///   영수증 쪽(`FlightTicket::split`)이 구조적으로 막고 있으므로, 만에 하나 어긋나면 **분모를 0 으로
    ///   수렴시키는 쪽**(= 유입을 막지 않는 쪽)이 안전하다 — 반대로 남겨 두면 그 수신자가 영영 메일을 못 받는다.
    pub fn settle_in_flight(&mut self, recipient: &str, ticket: FlightTicket) {
        if ticket.is_zero() {
            return;
        }
        let Some(slot) = self.in_flight.get_mut(recipient) else {
            return;
        };
        slot.message = slot.message.saturating_sub(ticket.message);
        slot.notice = slot.notice.saturating_sub(ticket.notice);
        if slot.is_zero() {
            self.in_flight.remove(recipient);
        }
    }

    /// 그 수신자의 현재 in-flight 총 건수(두 레인 합) = **지금 flush 가 락 밖으로 들고 나가 아직 정산되지
    ///   않은 몫**. 0 이면 "그 이름 앞으로 진행 중인 배치가 없다".
    ///
    /// ★운영 판정에 쓰인다(round-7 — 더 이상 테스트 전용이 아니다)★: ① FIFO 합류 판정의 in-flight 항
    ///   (`has_pending_ahead`) ② 같은 수신자에 대한 flush 중복 진입 차단(service.rs `flush_for` — 배치 B 가
    ///   배치 A 의 잔여를 앞지르는 것을 막는다). 테스트에서는 정산 누수 단언에도 쓴다.
    pub fn in_flight_len(&self, recipient: &str) -> usize {
        self.in_flight
            .get(recipient)
            .map(|t| t.message + t.notice)
            .unwrap_or(0)
    }

    /// TTL 초과 항목을 **전 수신자에 걸쳐** 걷어내 반환한다(상위가 각 항목을 장부에 `expired` 로 기록).
    ///
    /// ★반환 = 걷어낸 만료분 + **그 항목이 있던 수신자 큐 키**(`ExpiredParked`)★: 상위는 이 목록을 순회하며
    ///   ledger 를 `(msg_id, recipient)` 로 지목해 expired 전이를 남긴다(spec §5 "TTL 초과 expired, 장부
    ///   잔존"). recipient 를 함께 주는 이유 = 그룹 방송의 1:N 회계(`ExpiredParked` 주석 — 옛 msg_id 단독
    ///   역조회는 엉뚱한 멤버 레코드를 전이할 수 있었다). 비워진 큐는 맵에서 제거한다.
    /// ★순서★: 큐 간 순회 순서는 HashMap 이라 비결정이지만, **한 수신자 안에서는 오래된 순**을 보존한다
    ///   (상위 전이는 항목별 독립이라 큐 간 순서에 의존하지 않는다).
    /// ★전량 스캔이다 — front 조기 종료 안 한다(round-4 finding 1 · load-bearing)★: 옛 구현은 "FIFO 니까
    ///   front 가 미만료면 뒤도 미만료" 로 첫 미만료에서 break 했다. 그 전제는 **큐 정렬축(admission 순번)과
    ///   만료축(`parked_at`)이 같다** 는 가정인데, 둘은 같지 않다: ① `park` 의 `parked_at` 은 **호출자가 주는
    ///   값**이라 저장소가 단조성을 보장할 수 없다 ② `park_pending` 은 락 획득 **전에** 시각을 떠서 경합 시
    ///   수용 순서와 시각이 역전될 수 있다. 그 경우 더 최근 항목이 앞에 서면 **뒤에 있는 만료 항목이 sweep
    ///   에서 영구히 가려진다**(TTL 이 무력화되고 장부에도 안 남는다 = 조용한 유실의 다른 얼굴). 그래서 전량
    ///   스캔으로 바꿨다 — 비용은 큐 길이 선형이고 규모가 극소해(수신자당 cap 100, 큐 수는 소수) 무의미하다.
    ///   `drain` 도 같은 이유로 전량 분할이다(그쪽은 원래부터).
    /// ★순수★: 만료 판정은 인자 `now` 로만 한다(모듈 헤더 불변식).
    pub fn sweep_expired(&mut self, now: Instant) -> Vec<ExpiredParked> {
        let mut expired = Vec::new();
        // 큐를 순회하며 만료분을 분리. 남는 항목은 원래 상대 순서(admission 순번 증가)를 그대로 유지한다.
        self.queues.retain(|recipient, queue| {
            let scanned = std::mem::take(queue);
            for m in scanned {
                if m.is_expired(now) {
                    // 큐 키를 항목에 붙여 내보낸다 — 상위가 (msg_id, recipient) 로 정확히 전이하게(C4).
                    expired.push(ExpiredParked {
                        recipient: recipient.clone(),
                        msg: m,
                    });
                } else {
                    queue.push_back(m);
                }
            }
            !queue.is_empty()
        });
        expired
    }

    /// ★수신자 큐를 통째로 비워 항목을 돌려준다 — **프로필 삭제 정리 전용**(spec §5 · ADR-0116 결정 3)★.
    ///
    /// ★`drain` 과 다른 점(둘을 합치지 말 것)★: `drain` 은 "지금 주입할 몫을 꺼낸다" 라 만료/미만료를 갈라
    ///   돌려주고 되돌아올 여지를 남긴다. 이쪽은 **그 이름이 세상에서 사라졌다**는 통보라 갈래가 없다 —
    ///   전량을 종점 처리 대상으로 넘긴다(상위가 장부에 `failed` + `RECIPIENT_DELETED` 로 남긴다).
    /// ★반환값으로 유실을 막는다(이 모듈의 기존 규율)★: 저장소는 순수라 장부를 모른다. 조용히 비우면
    ///   "장부엔 `pending` 인데 큐엔 없는" 유령 pending 이 남으므로 항목째 돌려준다("산 메일 조용히 버리기
    ///   금지" — spec §5 불변식).
    /// ★notice 도 함께 치운다★: 그 이름 앞 통지도 받을 주체가 없어졌다 — 레인을 가리지 않는다(장부 종점은
    ///   상위가 같은 코드로 남긴다).
    /// ★in-flight 는 건드리지 않는다(load-bearing)★: 지금 flush 가 락 밖으로 들고 나간 몫은 **영수증
    ///   보유자의 것**이다(`FlightTicket` 계약). 여기서 회계를 손대면 그쪽 정산과 이중으로 어긋난다. 그
    ///   몫의 결말은 원래 경로가 정하고(배달 성공 또는 복원), 복원된 잔여는 **TTL 소관**이다 — 삭제 정리가
    ///   삭제 시점 단발이라는 spec §5 의 수용된 잔여와 같은 성질이다.
    // ADR-0116 (결정 3 — 삭제 정리)
    pub fn purge_recipient(&mut self, recipient: &str) -> Vec<ParkedMessage> {
        self.queues
            .remove(recipient)
            .map(|q| q.into_iter().collect())
            .unwrap_or_default()
    }

    /// 수신자 큐의 현재 항목 수(테스트·관측·열 큐 선정용). 큐가 없으면 0. message + notice 합.
    ///
    /// ★용량 회계와는 다르다★: cap 은 레인별 계수이고 in-flight 를 함께 본다(`park`/`can_admit`).
    pub fn len(&self, recipient: &str) -> usize {
        self.queues.get(recipient).map(|q| q.len()).unwrap_or(0)
    }

    /// ★FIFO 합류 판정의 단일 출처(round-7)★ — "지금 이 수신자 앞에 **먼저 나갈 게** 있나".
    ///   = 큐에 뭔가 남아 있거나 **또는** 그 이름 앞으로 나가 있는 in-flight 가 하나라도 있나.
    ///
    /// ★왜 in-flight 도 세나(round-7 high — 옛 결함의 정확한 모양)★: flush 는 큐를 통째로 비운 뒤 **락을
    ///   놓고** 주입한다. 그 구간의 큐는 비어 있지만 그 배치는 아직 수신자 stdin 에 닿지 않았다 — 그때
    ///   들어온 직발송이 큐만 보고 "앞에 없음" 으로 판정해 즉시 주입하면, 수신자는 (새것, 옛것들) 순서로
    ///   본다. "오래된 순 일괄" 은 큐 내부가 아니라 **그 수신자가 보는 순서**에 대한 약속이므로(ADR-0104)
    ///   이 창도 합류 대상이다. 이 결함은 C2/C3 의 flush 설계 이래 계속 있었고, round-7 의 in-flight
    ///   회계(cap 분모용으로 도입)가 비로소 **관측 수단**을 줬다.
    /// ★가시성 필터가 사라졌다(ADR-0111 결정 6)★: 옛 구현은 "다른 incarnation 앞으로 결박된 잔해" 를 큐
    ///   항에서 빼는 `visible_to` 필터를 걸었다. 결박이 폐지돼 모든 파킹분이 그 이름의 **누구에게든** 배달
    ///   가능해졌으므로 필터가 가릴 대상 자체가 없다 — 큐에 있으면 곧 앞에 서 있는 것이다(그래서 판정이
    ///   `len(...) > 0` 으로 단순해졌고, in-flight 항의 "과다 차단" 비대칭도 함께 소멸했다).
    /// ★kind 를 가르지 않는다★: 큐에 통지가 있어도 직발송은 합류한다(통지가 앞선 메일을 앞지르지 않는다는
    ///   FIFO 정합) — 그래서 in-flight 도 두 레인을 합쳐 본다.
    // ADR-0107 (FIFO 합류 = 큐 + in-flight)
    // ADR-0111 (결박 폐지 — 가시성 필터 제거)
    pub fn has_pending_ahead(&self, recipient: &str) -> bool {
        self.in_flight_len(recipient) > 0 || self.len(recipient) > 0
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
    /// ★왜 필요한가★: 파킹의 주소 축은 **이름**이라(service.rs `ResolvedRecipient.key`) busy 파킹은 **발송
    ///   시점의** canonical 이름 큐에 들어간다. 그런데 턴 종료 flush 는 그 에이전트의 **현재** 이름으로
    ///   진입하므로(tap 은 id 만 안다), 턴 중에 이름이 바뀌면(display_name 변경·cwd 파생 이름 변화) 옛 이름
    ///   큐를 아무도 열지 않아 그 배치가 TTL 까지 고아가 된다. 힌트로 역방향 조회를 하면 그 큐를 찾아낸다
    ///   (항목별 힌트 우선 해석은 flush 가 이미 하므로, 여기선 **어느 큐를 열어야 하나**만 답한다).
    /// ★비용(의도적 선택 — 인덱스 안 만든다)★: 전 큐 × 전 항목 선형 스캔이다. 규모가 극소하기 때문이다 —
    ///   큐는 파킹된 수신자 수(사람 대화 수준의 소수), 항목은 수신자당 cap 100. 별도 (id→큐) 인덱스를 두면
    ///   park/drain/restore_ordered/sweep 네 경로가 모두 인덱스를 정확히 갱신해야 하고, 한 곳만 놓쳐도 배달이
    ///   조용히 멈춘다(무손실 불변식과 정면 충돌). 유지 비용 대비 이득이 없어 스캔을 택했다.
    pub fn queues_with_hint(&self, id: PeerId) -> Vec<String> {
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

    /// 테스트 편의 — 회수가 없는 정상 park(대부분의 테스트가 레인 상한 아래에서 돈다).
    #[cfg(test)]
    fn park_ok(&mut self, recipient: &str, msg: ParkedMessage) -> Result<(), ParkError> {
        // 이 헬퍼를 쓰는 테스트는 전부 레인 상한 아래에서 돈다 — 회수가 일어났다면 그 테스트의 전제가
        //   깨진 것이므로 조용히 삼키지 않고 즉시 터뜨린다(회수 검증은 `park` 를 직접 부르는 전용 테스트).
        self.park(recipient, msg).map(|admitted| {
            assert!(
                admitted.retired.is_empty(),
                "park_ok 경로에서 예기치 않은 회수"
            );
        })
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

    /// 테스트용 파킹 항목 생성 — id·kind·park 시각을 지정한다(id 힌트 없음).
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
        let hint = PeerId::new_v4();
        let mut m = parked("m0", ParkKind::Message, t0);
        m.hinted_id = Some(hint);
        mb.park_ok("alice", m).expect("park");
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
        let target = PeerId::new_v4();
        let other = PeerId::new_v4();
        let mut hinted = parked("m0", ParkKind::Message, t0);
        hinted.hinted_id = Some(target);
        mb.park_ok("old-name", hinted).unwrap();
        let mut other_hint = parked("m1", ParkKind::Message, t0);
        other_hint.hinted_id = Some(other);
        mb.park_ok("someone-else", other_hint).unwrap();
        // 힌트 없는 파킹(테스트 seam 잔여)은 잡히지 않아야(그건 이름 규칙으로만 배달된다).
        mb.park_ok("absent-name", parked("m2", ParkKind::Message, t0))
            .unwrap();

        assert_eq!(
            mb.queues_with_hint(target),
            vec!["old-name".to_string()],
            "그 id 를 힌트로 든 큐만"
        );
        assert!(
            mb.queues_with_hint(PeerId::new_v4()).is_empty(),
            "무관한 id 는 빈 목록"
        );
        // drain 으로 비면 더 이상 잡히지 않는다(빈 큐는 맵에서 제거).
        let _ = mb.drain("old-name", t0);
        assert!(mb.queues_with_hint(target).is_empty());
    }

    #[test]
    fn a_queue_full_of_deliverable_items_rejects_instead_of_cannibalizing_live_mail() {
        // ★회수가 산 메일을 잡아먹지 않는다★: 큐가 **전부 배달 가능한** 항목이면 걷어낼 게 없으므로
        //   `MailboxFull` 반려로 끝난다(부작용 0).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park("w", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        assert_eq!(
            mb.park("w", parked("m-new", ParkKind::Message, t0)).err(),
            Some(ParkError::MailboxFull),
            "회수할 stale 이 없으면 반려"
        );
        assert_eq!(
            mb.len("w"),
            MAILBOX_CAP,
            "반려 경로는 큐를 건드리지 않는다(부작용 0)"
        );
        assert_eq!(mb.msg_ids("w")[0], "m0", "아무것도 걷어내지 않았다");
    }

    // ── round-6: notice 레인(독립 상한 + 반려 없음) ─────────────────────────────────────────
    #[test]
    fn the_notice_lane_retires_its_oldest_instead_of_rejecting() {
        // ★옛 "notice 는 cap 예외" 대체★: 면제는 곧 무계였다(근거로 들었던 MAX_OPEN_REQUESTS 는 notice
        //   수를 묶지 못한다 — due_timeouts 가 notified 를 즉시 세워 계약 자리를 비운다). 이제 자기 레인
        //   상한이 있고, 넘치면 **가장 오래된 통지**가 회수된다 — 반려는 없다(호출부가 결과를 버리므로).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..NOTICE_CAP {
            mb.park_ok("w", parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("{i}번째 notice(레인 이내)"));
        }
        assert_eq!(mb.len("w"), NOTICE_CAP);

        let admitted = mb
            .park("w", parked("n-new", ParkKind::Notice, t0))
            .expect("notice 는 절대 반려되지 않는다");
        assert_eq!(
            admitted
                .retired
                .iter()
                .map(|m| m.msg_id.as_str())
                .collect::<Vec<_>>(),
            vec!["n0"],
            "가장 오래된 통지 1건이 회수돼 장부에 남는다(조용한 유실 금지)"
        );
        let ids = mb.msg_ids("w");
        assert_eq!(ids.len(), NOTICE_CAP, "notice 레인은 상한을 넘지 않는다");
        assert_eq!(ids.first().unwrap(), "n1");
        assert_eq!(ids.last().unwrap(), "n-new");
    }

    #[test]
    fn the_two_lanes_never_block_each_other() {
        // ★독립 레인★: 가득 찬 message 백로그가 통지를 막지 않고, 쌓인 통지가 message 분모를 부풀리지도
        //   않는다(옛 구현은 후자만 성립 — 전자는 "면제" 로 흉내 냈고 그게 무계의 근원이었다).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park("w", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        // message 는 이제 반려되지만…
        assert_eq!(
            mb.park("w", parked("m-over", ParkKind::Message, t0)).err(),
            Some(ParkError::MailboxFull)
        );
        // …notice 는 자기 레인이 비어 있으므로 그대로 수용된다.
        for i in 0..NOTICE_CAP {
            let admitted = mb
                .park("w", parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("가득 찬 message 큐가 notice 를 막으면 안 된다({i})"));
            assert!(
                admitted.retired.is_empty(),
                "notice 레인은 아직 여유가 있다"
            );
        }
        assert_eq!(mb.len("w"), MAILBOX_CAP + NOTICE_CAP);
        // 역방향: notice 가 가득 차도 message 분모는 그대로다(여전히 message 사유로만 반려).
        assert_eq!(
            mb.park("w", parked("m-over2", ParkKind::Message, t0)).err(),
            Some(ParkError::MailboxFull),
            "notice 가 message 분모를 부풀리지 않는다(반려 사유는 message 백로그뿐)"
        );
        // 그리고 notice 레인이 가득 찬 상태에서도 통지는 계속 들어간다(회수하며).
        let admitted = mb
            .park("w", parked("n-new", ParkKind::Notice, t0))
            .expect("notice 반려 없음");
        assert_eq!(admitted.retired.len(), 1);
        assert_eq!(
            mb.len("w"),
            MAILBOX_CAP + NOTICE_CAP,
            "두 레인 합이 큐 총량의 상한이다(물리 상한 상수 불요)"
        );
    }

    // ── F1: 분모 = 큐 + in-flight(락 밖 배치) ─────────────────────────────────────────────────
    #[test]
    fn an_in_flight_batch_still_counts_against_the_cap() {
        // ★F1 회귀★: drain 이 큐를 비워도 그 배치는 여전히 "이 수신자 앞 미결 메시지" 다. 옛 구현은 분모가
        //   큐뿐이라, flush 가 락 밖에서 주입하는 동안 큐가 **비어 보여** 신규 유입이 cap 만큼 통째로 통과했다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok("w", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        let batch = mb.drain("w", t0).deliverable;
        assert_eq!(mb.len("w"), 0, "전제: drain 이 큐를 통째로 비웠다");
        let ticket = mb.take_in_flight("w", batch.iter());
        assert_eq!(mb.in_flight_len("w"), MAILBOX_CAP);

        assert_eq!(
            mb.park("w", parked("c0", ParkKind::Message, t0)).err(),
            Some(ParkError::MailboxFull),
            "큐가 비어 보여도 분모는 in-flight 를 센다(옛 구현 = 100건 더 수용)"
        );
        // 그 배치가 전부 배달돼 정산되면 자리가 다시 난다.
        mb.settle_in_flight("w", ticket);
        assert_eq!(mb.in_flight_len("w"), 0);
        mb.park("w", parked("c1", ParkKind::Message, t0))
            .expect("정산 뒤에는 수용");
        assert_eq!(mb.len("w"), 1);
    }

    // ── round-7: FIFO 합류 판정 = 큐 + in-flight ────────────────────────────────────────────────
    #[test]
    fn has_pending_ahead_sees_a_batch_that_already_left_the_queue() {
        // ★round-7 high 회귀(저장소 층)★: drain↔정산 사이의 큐는 **비어 있다**. 그 구간을 큐 길이
        //   로만 보면 "앞에 아무도 없다" 가 나와, 상위(직발송·방송)가 진행 중인 배치를 앞지른다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok("w", parked("m0", ParkKind::Message, t0))
            .expect("park");
        assert!(mb.has_pending_ahead("w"), "큐에 있으면 당연히 앞에 있다");

        let batch = mb.drain("w", t0).deliverable;
        assert_eq!(mb.len("w"), 0, "큐 항은 0(= 옛 판정의 전부)");
        let ticket = mb.take_in_flight("w", batch.iter());
        assert!(
            mb.has_pending_ahead("w"),
            "큐를 떠났어도 아직 수신자에게 닿지 않았다 — 앞에 있다"
        );

        mb.settle_in_flight("w", ticket);
        assert!(
            !mb.has_pending_ahead("w"),
            "정산되면(배달 완료) 비로소 앞이 빈다"
        );
    }

    #[test]
    fn repeated_drain_restore_cycles_never_grow_past_the_cap() {
        // ★F1 핵심 회귀(무계 성장)★: flush→inject 실패→복원 사이클을 반복해도 큐가 자라면 안 된다. 옛
        //   구현에선 사이클마다 "빈 큐 창" 으로 들어온 신규분 k 가 그대로 얹혀 매번 +k 로 자랐다(cap 무력화).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // 초기 큐 = cap 까지 **산 메일**(잔해가 아니라 회수로 가려지지 않는다 — 성장만 본다).
        for i in 0..MAILBOX_CAP {
            mb.park_ok("w", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        for cycle in 0..4 {
            let batch = mb.drain("w", t0).deliverable;
            let ticket = mb.take_in_flight("w", batch.iter());
            // drain↔복원 창의 동시 발송 — 옛 구현은 큐가 비어 보여 전부 통과했다.
            let admitted = (0..20)
                .filter(|i| {
                    mb.park("w", parked(&format!("c{cycle}-{i}"), ParkKind::Message, t0))
                        .is_ok()
                })
                .count();
            assert_eq!(
                admitted, 0,
                "사이클 {cycle}: 이미 cap 만큼 나가 있으므로 창이 열려 있어도 한 건도 못 받는다"
            );
            // inject 전량 실패 → 배치 그대로 복원 + **같은 자리에서** 정산(운영 flush 의 실패 갈래와 같은 짝).
            mb.restore_ordered("w", batch);
            mb.settle_in_flight("w", ticket);
            assert_eq!(
                mb.len("w"),
                MAILBOX_CAP,
                "사이클 {cycle}: 큐 + in-flight 합이 cap 을 넘지 않는다"
            );
        }
        assert_eq!(mb.in_flight_len("w"), 0, "정산 누수 없음");
    }

    #[test]
    fn a_partially_settled_ticket_frees_only_what_was_resolved() {
        // 배치가 항목별로 종점을 맞으면(배달 완료) 그만큼 분모가 즉시 풀려야 한다 — 배치 전체가 끝날 때까지
        //   유입을 통째로 막으면 정상 발송이 근거 없이 반려된다. 그리고 **이중 정산은 구조적으로 불가**해야
        //   한다(분모가 실제보다 작아지면 cap 이 뚫린다).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..3 {
            mb.park_ok("w", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        let batch = mb.drain("w", t0).deliverable;
        let mut ticket = mb.take_in_flight("w", batch.iter());
        assert_eq!(mb.in_flight_len("w"), 3);

        let one = ticket.split([ParkKind::Message]);
        mb.settle_in_flight("w", one);
        assert_eq!(mb.in_flight_len("w"), 2, "배달된 1건만 분모에서 빠진다");

        // 잔량(2)보다 많이 떼려 해도 2건까지만 나온다 — 이중 정산 방지.
        let rest = ticket.split([ParkKind::Message; 5]);
        mb.settle_in_flight("w", rest);
        assert_eq!(mb.in_flight_len("w"), 0);
        assert!(ticket.is_zero(), "영수증은 잔량 이상을 발급하지 않는다");
        // 과다 반납이 들어와도 분모가 음수로 감기지 않는다(saturating — settle_in_flight 주석).
        mb.settle_in_flight("w", mb_ticket_of(2));
        assert_eq!(mb.in_flight_len("w"), 0);
    }

    /// 위 테스트 전용 — 임의 건수의 message 영수증(과다 반납 방어 단언용).
    fn mb_ticket_of(n: usize) -> FlightTicket {
        let mut t = FlightTicket::default();
        for _ in 0..n {
            t.add(ParkKind::Message);
        }
        t
    }

    #[test]
    fn the_notice_lane_overshoot_while_a_batch_is_in_flight_is_exactly_one() {
        // notice 는 반려 갈래가 없으므로(회신 계약 통지가 막히면 계약이 반쪽) in-flight 로 레인이 찬 순간엔
        //   회수 대상을 큐에서 못 찾고도 수용된다. 그 초과는 **정확히 +1** 에서 멈춘다(NOTICE_CAP 주석 계약).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..NOTICE_CAP {
            mb.park_ok("w", parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        let batch = mb.drain("w", t0).deliverable;
        let ticket = mb.take_in_flight("w", batch.iter());
        let mut retired_total = 0usize;
        for i in 0..5 {
            let admitted = mb
                .park("w", parked(&format!("x{i}"), ParkKind::Notice, t0))
                .expect("통지는 절대 반려되지 않는다");
            retired_total += admitted.retired.len();
        }
        assert_eq!(
            mb.len("w") + mb.in_flight_len("w"),
            NOTICE_CAP + 1,
            "일시 초과는 +1 에서 멈춘다(다음 park 이 초과분 전체를 걷어낸다)"
        );
        assert_eq!(
            retired_total, 4,
            "밀려난 통지는 전부 반환돼 장부에 남는다(조용한 유실 0)"
        );
        mb.settle_in_flight("w", ticket);
        assert_eq!(mb.in_flight_len("w"), 0);
    }

    #[test]
    fn park_and_drain_preserves_fifo_order() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..5 {
            mb.park_ok("alice", parked(&format!("m{i}"), ParkKind::Message, t0))
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
        mb.park_ok("alice", parked("m0", ParkKind::Message, t0))
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
            mb.park_ok("bob", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째(cap 이내)는 수용해야"));
        }
        assert_eq!(mb.len("bob"), MAILBOX_CAP);
        // 101번째 message 는 반려.
        let over = mb.park_ok("bob", parked("overflow", ParkKind::Message, t0));
        assert_eq!(
            over,
            Err(ParkError::MailboxFull),
            "cap 초과 message 는 MailboxFull 반려"
        );
        assert_eq!(mb.len("bob"), MAILBOX_CAP, "반려된 항목은 큐에 안 들어감");
    }

    #[test]
    fn a_full_message_queue_still_accepts_notices() {
        // 레인 분리의 원래 목적(옛 "cap 예외" 가 지키려던 것): 가득 찬 메일박스가 회신 계약 통지를 막으면
        //   계약이 반쪽 난다(ADR-0103 불변식). 이제 통지는 **자기 레인**에서 그대로 수용된다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok("carol", parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        assert_eq!(
            mb.park_ok("carol", parked("msg-over", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull)
        );
        for i in 0..3 {
            mb.park_ok("carol", parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("가득 찬 message 큐가 notice 를 막으면 안 됨({i})"));
        }
        assert_eq!(mb.len("carol"), MAILBOX_CAP + 3);
        // notice 로 큐가 커져도 여전히 신규 message 는 message 사유로만 반려(분모는 레인별).
        assert_eq!(
            mb.park_ok("carol", parked("msg-over2", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull),
            "notice 가 message 분모를 부풀리면 안 됨"
        );
    }

    #[test]
    fn ttl_boundary_exactly_at_ttl_is_expired() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok("d", parked("m0", ParkKind::Message, t0))
            .unwrap();
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
        mb.park_ok("d", parked("m0", ParkKind::Message, t0))
            .unwrap();
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
        mb.park_ok("h", parked("old", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok(
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
        mb.park_ok("e", parked("old", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok(
            "e",
            parked("recent", ParkKind::Message, t0 + Duration::from_secs(1800)),
        )
        .unwrap();
        let now = t0 + PARK_TTL + Duration::from_nanos(1);
        let expired = mb.sweep_expired(now);
        assert_eq!(expired.len(), 1, "만료분 1건만 반환");
        assert_eq!(expired[0].msg.msg_id, "old");
        assert_eq!(
            expired[0].recipient, "e",
            "만료 항목이 있던 수신자 큐 키를 함께 돌려줘야(1:N 장부 전이 — C4)"
        );
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
        mb.park_ok("f", parked("only", ParkKind::Message, t0))
            .unwrap();
        let now = t0 + PARK_TTL + Duration::from_nanos(1);
        let expired = mb.sweep_expired(now);
        assert_eq!(expired.len(), 1);
        assert!(mb.is_empty(), "전부 만료돼 비면 큐가 맵에서 제거돼야");
    }

    #[test]
    fn sweep_expired_labels_each_item_with_its_own_recipient() {
        // ★C4 1:N 회계(ADR-0104 앵커 해소)★: 같은 msg_id 가 여러 멤버 큐에 파킹된 그룹 방송에서, 만료
        //   항목은 **자기 큐 키**를 달고 나와야 상위가 (msg_id, recipient) 로 정확히 전이한다. msg_id 만
        //   돌려주면 상위는 "이 msg_id 의 첫 pending 레코드" 를 찍을 수밖에 없어 엉뚱한 멤버가 만료된다.
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok("alice", parked("g1", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok("bob", parked("g1", ParkKind::Message, t0))
            .unwrap();
        let now = t0 + PARK_TTL;
        let mut expired = mb.sweep_expired(now);
        expired.sort_by(|a, b| a.recipient.cmp(&b.recipient)); // 큐 간 순서는 HashMap 이라 비결정.
        assert_eq!(expired.len(), 2, "두 멤버 큐의 같은 msg_id 가 각각 만료");
        assert_eq!(expired[0].recipient, "alice");
        assert_eq!(expired[1].recipient, "bob");
        assert!(
            expired.iter().all(|e| e.msg.msg_id == "g1"),
            "둘 다 같은 논리 메시지(1 msg_id : N 배달기록)"
        );
    }

    // ── restore_ordered(재파킹 무손실 복원 — finding 1 · round-4 finding 1) ─────────────
    #[test]
    fn restore_ordered_bypasses_cap_no_loss() {
        // ★조용한 유실 금지(ADR-0103 · finding 1)★: 큐가 이미 cap(100) 이면 park 는 반려하지만, restore_ordered
        //   는 cap 을 우회해 admitted 항목을 무조건 되돌린다(유령 pending 방지).
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok("r", parked(&format!("new{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        assert_eq!(mb.len("r"), MAILBOX_CAP);
        // park 는 이제 반려된다(cap 도달).
        assert_eq!(
            mb.park_ok("r", parked("would-reject", ParkKind::Message, t0)),
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
        mb.park_ok(
            "r",
            parked("new0", ParkKind::Message, t0 + Duration::from_secs(10)),
        )
        .unwrap();
        mb.park_ok(
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
            mb.park_ok("dup", parked(&format!("m{i}"), ParkKind::Message, t0))
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
        mb.park_ok("dup", parked("new", ParkKind::Message, t0))
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
            mb.park_ok("r", parked(&format!("old{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        let drained = mb.drain("r", t0).deliverable; // old0..old2 (순번 0..2)
        mb.park_ok("r", parked("new0", ParkKind::Message, t0))
            .unwrap(); // 순번 3
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
        mb.park_ok(
            "z",
            parked("newer", ParkKind::Message, t0 + Duration::from_secs(3600)),
        )
        .unwrap();
        mb.park_ok("z", parked("older", ParkKind::Message, t0))
            .unwrap();
        let now = t0 + PARK_TTL;
        let expired = mb.sweep_expired(now);
        assert_eq!(
            expired
                .iter()
                .map(|e| e.msg.msg_id.as_str())
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
        mb.park_ok("g", parked("first", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok(
            "g",
            parked("second", ParkKind::Message, t0 + Duration::from_secs(1)),
        )
        .unwrap();
        mb.park_ok(
            "g",
            parked("third", ParkKind::Message, t0 + Duration::from_secs(2)),
        )
        .unwrap();
        let now = t0 + PARK_TTL + Duration::from_secs(10);
        let expired = mb.sweep_expired(now);
        let ids: Vec<&str> = expired.iter().map(|e| e.msg.msg_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["first", "second", "third"],
            "sweep 도 오래된 순 보존"
        );
    }
}
