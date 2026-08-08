//! busy — 수신자 턴 상태 해석 + idle 게이트 seam(ADR-0104 결정 3 · ADR-0113 결정 2).
//!
//! ★역할 = **정책**뿐★: "지금 턴 중인가" 라는 **사실**은 호스트의 공용 관측 계층이 들고 있고(ADR-0113
//!   결정 1), 이 모듈은 그 사실을 **우편의 가치판단으로 해석**한다. 조각 셋:
//!     ① `TurnFacts` — 사실을 읽어 오는 포트(호스트 어댑터가 구현).
//!     ② `BusyPolicy` — 그 사실 + 이 모듈의 정책(positive-knowledge-only · `BUSY_MAX_TURN` 상한)으로
//!        `BusyGate` 답을 만들고, 상한을 넘긴 턴을 도어벨로 깨운다.
//!     ③ `BusyGate`/`IdleNotifier` — 서비스가 묻는 문과 flush 도어벨 출구.
//!
//! ★positive-knowledge-only(load-bearing — spec §5 capability 폴백)★: 관측이 **없는** (id, epoch) 는 전부
//!   **idle 취급**이다(= 즉시 주입). "모른다" 를 busy 로 해석하면 관측 불가 백엔드·관측이 아직 시작되지
//!   않은 창에서 배달이 **영구 대기**한다. busy 는 **관측된 사실이 있을 때만** 참이다.
//!
//! ★사실 계층을 **변형하지 않는다**(ADR-0113 결정 2 — 절대 위반 금지)★: 상한 판정도, sweep 도 표에서
//!   항목을 지우지 않는다. 그 표는 다른 소비자(UI 입력 잠금 등)와 공유하는 공용 시설이고, 소비자마다
//!   상한이 다르다 — 한 소비자가 "늙었다" 고 지우면 다른 소비자의 사실이 증발한다. 여기서 하는 일은
//!   **읽고 자기 기준으로 판정**하는 것뿐이다.
//!
//! ★busy 관측 대상 = 턴 신호를 내는 백엔드★: 턴 이벤트는 백엔드 decoder 가 있는
//!   에이전트에서만 나오고, decoder 는 구조화 출력 capability 와 **정확히 같은 조건**으로 존재한다.
//!   그 프록시 값은 `LiveAgent::turn_signal` 로 로스터 항목에 실려 오고(ADR-0116 결정 7), 발송 경로는
//!   `turn_signal == false` 인 부류에 이 게이트를 **아예 묻지 않고** 즉시 주입한다. 프록시가 깨지는 날
//!   (구조화인데 턴 이벤트가 없는 백엔드 등장) 진짜 capability 필드를 추가한다 — ADR-0104 capability
//!   원칙과 정합(관측 불가 = 즉시 주입 폴백).
//!
//! ★콜백 규율(load-bearing — 절대 위반 금지)★: `IdleNotifier::notify_idle` 은 **호스트의 출력 pump
//!   스레드**에서도 불린다. 그 구현이 하는 일은 논블록 통지 send **하나뿐**이어야 한다 — 주입·messaging
//!   락 취득·blocking write 를 하면 출력 pump 가 배달 작업 뒤에서 막혀 전 에이전트의 출력 스트림이
//!   지연된다. 실제 flush 는 통지를 받은 flush worker 가 수행한다.
//!
//! ★알려진 미확인(측정 항목 — spec §7)★: 중첩 Task 서브에이전트의 `result` 라인이 **부모 턴 종료**
//!   신호로 새는지 미검증이다. 새면 부모가 아직 턴 중인데 idle 로 오판해 조기 주입할 수 있다
//!   (유실은 없고 타이밍만 어긋남). 해결하지 않고 실 하네스 측정 항목으로 남긴다.
// ADR-0104
// ADR-0110
// ADR-0113

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::PeerId;

/// ★busy 상한(fail-open 안전 밸브 — round-3 finding 4, 내부 선택: **30분**)★: 마지막 턴 신호로부터 이
///   시간을 넘긴 "턴 중" 관측은 **비정상 종료된 턴의 잔해**로 보고 idle 로 판정한다.
///
/// ★왜 상한이 필요한가★: 턴 종료 신호는 유일한 in-band 해제다. 턴이 비정상 종료(파싱 실패·decoder
///   이상·종료 라인 누락)하면 그 수신자 앞 모든 배달이 도어벨마다 접히고 결국 TTL 로 만료된다
///   ("안 가는 것" = 메시징 최악 실패 모드). 발생 빈도는 미측정(spec §7 항목).
/// ★왜 상한이 안전한가(무엇을 잃나)★: 오판(실제로는 턴 중인데 idle 로 봄)의 결과는 **턴 중 주입**이고,
///   claude CLI 는 턴 중 stdin 을 내부 큐에 넣어 **다음 턴에 읽는다**(실측된 사용자 대면 동작 — spec §7).
///   즉 최악의 대가는 "언제 읽히나" 가 흐려지는 것뿐이며 **유실은 없다**. 반면 상한이 없으면 그 수신자 앞
///   메일이 TTL 까지 전부 안 간다 — 비교 불가하게 나쁘다(ADR-0104 "늦게 가는 것 < 안 가는 것").
/// ★왜 30분인가★: 사람 대화 수준 메시지율에서 30분 무-출력 턴은 정상 범위를 크게 벗어난다(도구 호출·
///   delta·usage 중 하나라도 오면 사실 계층이 시각을 갱신하므로, 30분은 "출력이 완전히 멈춘 채 턴 종료
///   신호도 없는" 구간을 뜻한다). 더 짧으면 정상 장기 턴을 자르고, 더 길면 회복이 늦다.
pub const BUSY_MAX_TURN: Duration = Duration::from_secs(30 * 60);

/// ★idle 게이트 조회 seam(ADR-0012)★ — MessagingService 가 "이 수신자가 지금 턴 중인가" 를 묻는 유일한 문.
///
/// ★계약★: 순수 조회 — 부작용 없음, 블로킹 없음(짧은 락만). messaging 락을 **든 채** 불려도 안전해야
///   한다(현 호출부는 락 밖에서 부르지만, 이 계약을 지켜 두면 미래 호출 지점이 늘어도 데드락이 없다).
pub trait BusyGate: Send + Sync {
    fn is_busy(&self, id: PeerId, epoch: u32) -> bool;
}

/// 게이트 미배선/관측 불가 폴백 — **항상 idle**(= 즉시 주입, spec §5 capability 폴백).
pub struct AlwaysIdleGate;

impl BusyGate for AlwaysIdleGate {
    fn is_busy(&self, _id: PeerId, _epoch: u32) -> bool {
        false
    }
}

/// 한 화신의 턴 관측 사실 — 해석이 붙지 않은 원값(ADR-0113 결정 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnFact {
    pub in_turn: bool,
    pub last_signal: Instant,
}

/// ★턴 사실 조회 포트(ADR-0110 결정 3 · ADR-0113 결정 1)★ — 호스트의 공용 관측 계층을 커널이 **타입으로도
///   모른 채** 읽는 문. 운영 구현은 호스트 어댑터가 소유한다.
///
/// ★읽기 전용이 계약이다★: 이 포트에 "지워라/표시해라" 를 추가하지 말 것(근거 = 모듈 헤더).
/// ★두 값을 한 번에 돌려주는 이유★: `in_turn` 과 `last_signal` 을 따로 물으면 두 조회 사이에 신호가 끼어
///   "턴 중인데 시각은 옛것" 같은 합성 불가능한 조합으로 판정하게 된다.
pub trait TurnFacts: Send + Sync {
    /// 이 (id, epoch)의 관측값. `None` = 미관측.
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact>;

    /// 지금 턴 중으로 관측된 전원 `(id, epoch, 마지막 신호 시각)` — 상한 sweep 의 입구.
    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)>;
}

/// ★턴 종료(idle 전이) 통지 seam★ — 호스트가 "이 에이전트 턴이 끝났다" 를 알리는 입구이자, 상한 sweep 이
///   멈춘 턴을 깨우는 출구. 운영 구현은 flush worker 채널로 논블록 send 한다.
///
/// ★계약(load-bearing)★: **논블록·비-재진입**(근거 = 모듈 헤더 콜백 규율).
/// ★idempotency 의존(수용된 설계)★: 통지는 **턴 종료 신호마다** 나간다(전이 여부를 따지지 않는다). 왜:
///   "직전이 busy 였을 때만" 으로 좁히면, 어시스턴트 이벤트 없이 곧장 끝나는 턴에서 통지가 빠져 파킹이
///   다음 턴까지 stranded 된다 — 배달 누락(치명)보다 **잉여 통지(무해)** 를 택한다. 잉여 통지의 대가는
///   빈 큐 drain no-op 뿐이다. 채널 압력의 상한은 호스트 통지 구현이 책임진다.
pub trait IdleNotifier: Send + Sync {
    fn notify_idle(&self, id: PeerId);
}

/// ★BusyPolicy — 사실 위에 우편의 정책을 얹는 게이트(C2 · ADR-0113 결정 2)★. 데몬 부팅에서 하나 만들어
///   MessagingService(게이트 조회)와 sweep task(상한 fail-open)가 공유한다.
///
/// ★유일한 상태 = 상한 판정 장부이고 그건 **정책** 상태다★: 사실은 전부 포트 너머에 있다. 공용 표를 못
///   지우므로(모듈 헤더) "이 화신의 이 신호는 잔해로 본다" 는 판정을 이쪽에 적어 둔다 — 그게 곧 게이트의
///   답이자 "이미 깨웠다" 표시다.
/// ★시계를 읽지 않는다(결정론 유지)★: 상한 비교는 `now` 를 인자로 받는 `sweep_stale_busy` 안에서만
///   일어나고, 조회(`is_busy`)는 그 장부를 볼 뿐이다. 그래서 crate 순수성 규율(clock injection)이 이
///   모듈에서도 성립하고, 판정 시점이 sweep 주기에 고정돼 관측 가능한 동작이 결정적이다.
/// ★락 = 이 장부 하나, 통지는 락 밖(ADR-0006)★: `notify_idle` 은 외부 호출이라 락을 든 채 부르지 않는다.
pub struct BusyPolicy {
    facts: Arc<dyn TurnFacts>,
    notifier: Arc<dyn IdleNotifier>,
    /// 상한 초과로 **잔해 판정**한 화신 → 그때 본 마지막 신호 시각. 값이 어긋나면(새 신호가 왔다) 판정은
    /// 무효다 — 그래서 다음 신호가 오는 순간 sweep 을 기다리지 않고 다시 busy 로 돌아간다.
    stale: Mutex<HashMap<(PeerId, u32), Instant>>,
}

impl BusyPolicy {
    pub fn new(facts: Arc<dyn TurnFacts>, notifier: Arc<dyn IdleNotifier>) -> Self {
        Self {
            facts,
            notifier,
            stale: Mutex::new(HashMap::new()),
        }
    }

    /// ★상한 sweep(round-3 finding 4)★ — `BUSY_MAX_TURN` 을 넘긴 "턴 중" 관측을 **잔해로 판정**하고 그
    ///   주인을 **도어벨로 깨운다**(대기 중인 파킹이 그때 배달된다). 데몬의 주기 sweep task 가 부른다.
    ///   반환 = 이번에 깨운 id 수.
    ///
    /// ★깨우기가 fix 의 절반이다★: 판정만 뒤집으면 그 수신자 앞 파킹은 **다음 트리거**(등장/턴 종료/새 발송)
    ///   까지 그대로 앉아 있다 — 그런데 애초에 이 상황은 "그 트리거가 오지 않는" 상황이다(턴 종료 신호 유실).
    /// ★잔해 하나당 도어벨 1회★: 사실을 지울 수 없으므로 같은 잔해가 매 주기 다시 보인다 — 마지막 신호
    ///   시각이 장부와 같으면 이미 처리한 잔해다. 새 신호가 왔다가 다시 멈추면 그건 새 잔해다.
    /// ★장부는 매번 "지금 잔해인 것" 으로 갈아 끼운다★: 되살아났거나 reap 된 화신의 항목이 쌓이지 않는다.
    pub fn sweep_stale_busy(&self, now: Instant) -> usize {
        let stale_now: Vec<(PeerId, u32, Instant)> = self
            .facts
            .in_turn_snapshot()
            .into_iter()
            .filter(|(_, _, last)| now.saturating_duration_since(*last) >= BUSY_MAX_TURN)
            .collect();

        let mut to_wake: Vec<PeerId> = Vec::new();
        {
            let mut ledger = self.stale.lock().expect("busy stale ledger poisoned");
            let mut next: HashMap<(PeerId, u32), Instant> = HashMap::new();
            for (id, epoch, last) in stale_now {
                if ledger.get(&(id, epoch)) != Some(&last) && !to_wake.contains(&id) {
                    to_wake.push(id);
                }
                next.insert((id, epoch), last);
            }
            *ledger = next;
        }

        for id in &to_wake {
            tracing::warn!(
                agent = %id,
                max_secs = BUSY_MAX_TURN.as_secs(),
                "busy 상한 초과 — 턴 종료 관측 없이 늙은 턴을 idle 로 판정하고 flush 도어벨(fail-open 안전 밸브)"
            );
            self.notifier.notify_idle(*id);
        }
        to_wake.len()
    }

    pub fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
        let Some(fact) = self.facts.turn_fact(id, epoch) else {
            return false;
        };
        if !fact.in_turn {
            return false;
        }
        let ledger = self.stale.lock().expect("busy stale ledger poisoned");
        ledger.get(&(id, epoch)) != Some(&fact.last_signal)
    }
}

impl BusyGate for BusyPolicy {
    fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
        BusyPolicy::is_busy(self, id, epoch)
    }
}

/// ★하네스/테스트 전용 사실 소스★ — 호스트 관측 계층 대신 손으로 사실을 심어 정책을 결정적으로 구동한다.
#[cfg(any(test, feature = "test-harness"))]
#[derive(Default)]
pub struct ScriptedTurnFacts {
    facts: Mutex<HashMap<(PeerId, u32), TurnFact>>,
}

#[cfg(any(test, feature = "test-harness"))]
impl ScriptedTurnFacts {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_in_turn(&self, id: PeerId, epoch: u32, at: Instant) {
        self.facts.lock().unwrap().insert(
            (id, epoch),
            TurnFact {
                in_turn: true,
                last_signal: at,
            },
        );
    }

    /// 이 화신을 "턴 끝남, 마지막 신호 = `at`" 으로 심는다(관측은 있으나 턴 중은 아님).
    pub fn set_idle(&self, id: PeerId, epoch: u32, at: Instant) {
        self.facts.lock().unwrap().insert(
            (id, epoch),
            TurnFact {
                in_turn: false,
                last_signal: at,
            },
        );
    }

    pub fn forget(&self, id: PeerId, epoch: u32) {
        self.facts.lock().unwrap().remove(&(id, epoch));
    }
}

#[cfg(any(test, feature = "test-harness"))]
impl TurnFacts for ScriptedTurnFacts {
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact> {
        self.facts.lock().unwrap().get(&(id, epoch)).copied()
    }
    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)> {
        self.facts
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, f)| f.in_turn)
            .map(|((id, epoch), f)| (*id, *epoch, f.last_signal))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct RecordingNotifier {
        seen: StdMutex<Vec<PeerId>>,
    }
    impl RecordingNotifier {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: StdMutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<PeerId> {
            self.seen.lock().unwrap().clone()
        }
    }
    impl IdleNotifier for RecordingNotifier {
        fn notify_idle(&self, id: PeerId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    fn policy() -> (BusyPolicy, Arc<ScriptedTurnFacts>, Arc<RecordingNotifier>) {
        let facts = ScriptedTurnFacts::new();
        let notifier = RecordingNotifier::new();
        (
            BusyPolicy::new(facts.clone(), notifier.clone()),
            facts,
            notifier,
        )
    }

    #[test]
    fn unknown_agent_is_idle_positive_knowledge_only() {
        let (p, _f, _n) = policy();
        assert!(
            !p.is_busy(PeerId::new_v4(), 0),
            "미관측 대상은 idle 취급(관측 불가 백엔드 즉시 주입 폴백)"
        );
    }

    #[test]
    fn an_observed_turn_is_busy_and_its_end_is_idle() {
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        assert!(p.is_busy(id, 0));
        f.set_idle(id, 0, t0);
        assert!(!p.is_busy(id, 0), "턴 종료 관측 → idle");
    }

    #[test]
    fn state_is_keyed_by_epoch_no_leak_across_incarnations() {
        // 옛 epoch 의 관측이 새 epoch 판정에 새면 새 incarnation 앞 메일이 근거 없이 대기한다.
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        f.set_in_turn(id, 0, Instant::now());
        assert!(p.is_busy(id, 0));
        assert!(!p.is_busy(id, 1), "다른 epoch 은 미관측 → idle");
    }

    #[test]
    fn the_ceiling_flips_the_verdict_without_touching_the_facts() {
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN - Duration::from_secs(1));
        assert!(
            p.is_busy(id, 0),
            "상한 안쪽은 여전히 busy(정상 장기 턴을 자르면 턴 중 주입이 된다)"
        );
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN);
        assert!(!p.is_busy(id, 0), "상한 초과는 잔해로 보고 idle");
        assert_eq!(
            f.turn_fact(id, 0),
            Some(TurnFact {
                in_turn: true,
                last_signal: t0
            }),
            "사실은 그대로 — 우편의 판정이 공용 표를 바꾸지 않는다"
        );
    }

    #[test]
    fn a_fresh_turn_signal_restores_busy_without_waiting_for_a_sweep() {
        let (p, f, _n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        p.sweep_stale_busy(t0 + BUSY_MAX_TURN);
        assert!(!p.is_busy(id, 0));
        f.set_in_turn(id, 0, t0 + Duration::from_secs(31 * 60));
        assert!(p.is_busy(id, 0), "새 신호가 오면 잔해 판정이 무효화된다");
    }

    #[test]
    fn sweep_wakes_only_stale_turns() {
        let (p, f, n) = policy();
        let stale = PeerId::new_v4();
        let fresh = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(stale, 0, t0);
        f.set_in_turn(fresh, 0, t0 + Duration::from_secs(25 * 60));
        assert_eq!(
            p.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            1
        );
        assert_eq!(n.seen(), vec![stale]);
        assert!(p.is_busy(fresh, 0), "신선한 턴 판정은 보존");
    }

    #[test]
    fn sweep_does_not_wake_before_the_ceiling() {
        let (p, f, n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        assert_eq!(
            p.sweep_stale_busy(t0 + BUSY_MAX_TURN - Duration::from_secs(1)),
            0,
            "상한 미달은 잔해가 아니다"
        );
        assert!(n.seen().is_empty());
    }

    #[test]
    fn the_same_stale_turn_is_woken_once_but_a_new_one_wakes_again() {
        let (p, f, n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        let late = t0 + BUSY_MAX_TURN + Duration::from_secs(1);
        assert_eq!(p.sweep_stale_busy(late), 1);
        assert_eq!(p.sweep_stale_busy(late + Duration::from_secs(60)), 0);
        assert_eq!(n.seen(), vec![id], "도어벨 1회");

        let t1 = late + Duration::from_secs(120);
        f.set_in_turn(id, 0, t1);
        assert_eq!(p.sweep_stale_busy(t1 + BUSY_MAX_TURN), 1);
        assert_eq!(n.seen(), vec![id, id]);
    }

    #[test]
    fn sweep_forgets_incarnations_that_left_the_facts() {
        let (p, f, n) = policy();
        let id = PeerId::new_v4();
        let t0 = Instant::now();
        f.set_in_turn(id, 0, t0);
        let late = t0 + BUSY_MAX_TURN + Duration::from_secs(1);
        assert_eq!(p.sweep_stale_busy(late), 1);
        f.forget(id, 0);
        assert_eq!(p.sweep_stale_busy(late), 0);
        f.set_in_turn(id, 0, t0);
        assert_eq!(p.sweep_stale_busy(late), 1);
        assert_eq!(n.seen(), vec![id, id]);
    }

    #[test]
    fn always_idle_gate_never_reports_busy() {
        let g = AlwaysIdleGate;
        assert!(!g.is_busy(PeerId::new_v4(), 7));
    }
}
