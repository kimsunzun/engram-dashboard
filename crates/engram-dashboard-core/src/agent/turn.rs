//! turn — (에이전트, epoch)별 **턴 진행 관측 표**. 코어가 소유하는 에이전트 "사실" 계층
//! (ADR-0113 결정 1 · ADR-0119 결정 4 — 명부 옆자리).
//!
//! ★이 모듈이 소유하는 것 = 관측된 사실뿐★: "이 화신이 지금 턴 중인가" 와 "마지막 턴 신호가 언제였나".
//!   폴백·상한·도어벨 같은 **해석은 소비자 몫**이다(ADR-0113 결정 2) — 소비자마다 오판 비용이 다르다
//!   (우편은 "안 가는 것" 이 최악이라 idle 쪽으로 무너지는 폴백을 쓰고, 입력 잠금은 다른 상한을 원한다).
//!   여기에 정책을 넣으면 한 소비자의 가치판단이 전원에게 강제된다.
//!
//! ★이벤트→신호 **매핑은 여기 없다**(ADR-0004/0110 결정 4 의 취지)★: 어떤 출력 이벤트가 턴 진행인지는
//!   백엔드 지식이라 `backend::AgentBackend::turn_classifier` 가 소유한다. 이 모듈이 아는 것은 신호 어휘
//!   (`TurnSignal`)와 그 신호를 쌓는 방법뿐이다.
//!
//! ★쓰기 경로 = `OutputCore::emit` 하나지만 그 **호출자는 둘**이다(load-bearing)★: ① 출력 pump 스레드
//!   ② 입력 에코 경로(`AgentSession::write_input_observed` — 주입한 스레드가 그대로 emit 한다: 우편
//!   flush 레인의 blocking 스레드·MCP HTTP 핸들러·Tauri 커맨드 스레드). 그래서 "단일 스레드" 를 전제로
//!   한 논증은 성립하지 않는다. 대신 성립하는 것:
//!     - 이 표의 뮤텍스는 **leaf** 다 — 다른 락과 함께 잡지 않고, 잡은 채 외부를 부르지 않는다. 그래서
//!       어느 스레드에서 불려도 락 순서 규약(ADR-0006)에 순환을 만들지 않는다.
//!     - 임계구역은 해시맵 조작 몇 줄뿐이다. 이건 호출자 중 하나가 **출력 pump** 라서 필수다 — 여기가
//!       길어지면 그 에이전트의 출력 스트림 전체가 뒤에서 밀린다.
//!
//! ★락 = 이 표 하나뿐(순서 규약이 필요 없는 구조)★: 항목을 `AgentId` 로 키잡고 epoch 을 **값**으로 들어
//!   맵 하나로 끝낸다. epoch 을 키에 넣으면 "이 id 의 현재 epoch 은 무엇인가" 를 답할 두 번째 표가 필요해지고
//!   그 순간 두 락 사이 순서 규약이 생긴다.
//!
//! ★한 id 에 항목 하나★: 살아 있는 화신은 언제나 하나다. 그 자리를 차지하는 유일한 동사가 [`TurnObservations::register`]
//!   (주인 = `AgentManager::spawn_session`)이고, 항목의 주 청소 경로는 `forget`(주인 = `OutputCore::finish`)다.
//!   ★화신 표식은 **서로 다르다**는 것만 보장한다 — 순서는 뜻이 없다★: 모든 spawn 이 모드와 무관하게
//!   `AgentManager::spawn_agent` 의 단일 발급 지점(`ProfileRegistry::epoch_for_spawn`)을 지나고, 거기서
//!   화신마다 새 난수를 뽑아 직전 값과 다름을 보장한다. **호출부에 발급을 흩뿌리지 말 것** — 그렇게 하던
//!   시절에 Fresh 재spawn 경로들이 표식을 재사용해 여기 두 화신이 같은 키로 앉았다(그 회귀가 이 문단의
//!   존재 이유다).
//!   ★그래서 표식 비교는 **일치/불일치**뿐이다 — 대소를 묻지 않는다(되살리지 마라)★: 한때 `observe_at` 이
//!   "더 작은 표식이면 버린다" 로 지각 신호를 걸렀다. 표식이 순서 있는 카운터였을 땐 그게 100% 발화했지만
//!   난수로 바뀐 뒤로는 **죽어가는 화신의 표식이 절반의 확률로 더 커서** 그대로 통과했고, 그 신호가 산
//!   화신의 항목을 덮었다. 그 뒤 산 화신의 조회는 미관측(=턴 아님)으로 답해 **턴 중에 우편이 들어간다.**
//!   자리를 바꾸는 권한을 `register` 하나로 좁힌 것이 그 구멍을 닫는다 — 지각 신호는 표식이 안 맞아 버려진다.
// ADR-0113
// ADR-0006

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::agent::types::AgentId;

/// 출력 이벤트에서 읽어낸 턴 신호 — 표가 아는 어휘 전부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSignal {
    /// 그 이벤트를 낸 백엔드가 "이 화신은 턴 진행 중" 으로 분류했다.
    Progress,
    /// 그 이벤트를 낸 백엔드가 "이 화신의 턴이 끝났다" 로 분류했다.
    Ended,
}

/// 한 (에이전트, epoch)의 턴 관측 스냅샷. 부재(`Option::None`)는 **미관측**이지 idle 이 아니다 —
/// 둘을 어떻게 취급할지는 소비자가 정한다(모듈 헤더).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnObservation {
    pub in_turn: bool,
    /// 이 화신에 대해 마지막으로 턴 신호를 관측한 시각. 소비자가 "이 관측이 얼마나 늙었나" 를 자기
    /// 기준으로 판정하는 축이다(멈춘 턴 탐지 등).
    pub last_signal: Instant,
}

/// 턴 관측 표 — `AgentManager` 가 하나 소유하고 그 매니저의 모든 `OutputCore` 가 공유한다.
///
/// ★관측 대상 = "턴 신호를 실제로 내는 세션" 이다(별도 capability 게이트가 없는 이유)★: 신호를 낼지
///   말지는 그 세션의 백엔드 분류자가 정한다(`backend::turn_classifier` — 선언 안 한 백엔드는 침묵).
///   그래서 "구조화 출력 capability" 로 여기서 다시 거르지 않는다 — 그러면 코어가 세션 조립 정보를
///   되짚어야 하고, 두 기준이 갈리는 날 어느 쪽이 정본인지 알 수 없게 된다.
pub struct TurnObservations {
    entries: Mutex<HashMap<AgentId, Entry>>,
}

struct Entry {
    epoch: u32,
    in_turn: bool,
    last_signal: Instant,
    /// 이 항목을 마지막으로 갱신한 **출력 seq**(같은 epoch 안에서만 비교 가능 — 화신이 바뀌면 seq 는 0
    /// 부터 다시 센다). 발행 순서와 적용 순서가 갈릴 때 늦은 신호를 걸러내는 축이다(`observe_at`).
    last_seq: u64,
}

impl Default for TurnObservations {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnObservations {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// ★poison 내성(reaper.rs 의 sessions 락과 같은 판단)★: 이건 **매니저 전역** 표라 어느 홀더가
    ///   패닉하면 그 뒤 **모든** 에이전트의 턴 신호 emit 이 연쇄로 재패닉한다 — core 의 에이전트 전용
    ///   락(`expect` 정당화가 "다른 agent 로 전파 안 됨" 인 것들)과 성질이 다르다. 담긴 건 해시맵뿐이라
    ///   불변식이 깨진 게 아니므로 가드를 회수해 계속 돈다. (release 는 panic=abort 라 이 내성이 실제로
    ///   의미 있는 건 debug/테스트 빌드다.)
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 이 화신이 이 id 의 자리를 차지한다 — 있던 항목이 무엇이든 **무조건** 갈아치운다.
    ///
    /// ★호출자는 `AgentManager::spawn_session` 하나이고 호출 시점은 **세션이 관측 가능해지기 전**이다
    ///   (sessions 맵 insert 전 = pump 기동 전). 이 순서가 이 표의 정확성을 떠받친다★: 등록이 그 화신의
    ///   첫 신호보다 늦으면 그 신호는 옛 항목의 표식과 안 맞아 버려지고, 그 화신은 옛 항목이 `forget` 될
    ///   때까지 **영구 미관측**(=턴 아님)으로 답한다 — 턴 중 우편 주입이 그 결말이다.
    /// ★죽어가는 앞 화신과의 순서는 어느 쪽이든 안전하다★: 앞 화신의 `forget` 이 먼저면 항목이 비고 우리가
    ///   채운다. 우리가 먼저면 앞 화신의 `forget` 은 표식이 달라 no-op 이고, 뒤늦은 신호도 같은 이유로 버려진다.
    /// ★넣는 값은 "아직 신호 없음"★: `in_turn=false` 다. 표는 사실만 담으므로 등록 자체가 턴을 뜻하지 않는다.
    ///   그래서 신호를 하나도 내지 않는 백엔드의 항목은 `in_turn_snapshot` 에 실리지 않고 `finish` 가 거둔다.
    pub fn register(&self, id: AgentId, epoch: u32) {
        self.register_at(id, epoch, Instant::now());
    }

    /// 시각 주입형 [`Self::register`].
    pub fn register_at(&self, id: AgentId, epoch: u32, at: Instant) {
        self.lock().insert(
            id,
            Entry {
                epoch,
                in_turn: false,
                last_signal: at,
                last_seq: 0,
            },
        );
    }

    /// 턴 신호 1건 반영(관측 시각 = 지금).
    /// `seq` = 그 이벤트의 출력 시퀀스(발행 순서의 정본 — `observe_at` 의 순서 역전 방어축).
    pub fn observe(&self, id: AgentId, epoch: u32, seq: u64, signal: TurnSignal) {
        self.observe_at(id, epoch, seq, signal, Instant::now());
    }

    /// 시각 주입형 — 테스트가 상한 경계·시각 갱신·순서 역전을 실시간 대기 없이 구동한다.
    ///
    /// ★옛 화신의 지각 신호를 거르려는 자리다(보조 방어)★: 화신 교체 직후에도 옛 `OutputCore` 는 큐에
    ///   남은 이벤트를 잠시 더 배출한다(pump 가 EOF 로 끝나기 전). 그 신호를 받으면 이미 사라질 운명인
    ///   화신의 항목이 되살아나는데, 그 화신의 종료 신호는 **영영 오지 않으므로**(이미 지나간 스트림)
    ///   아무도 지우지 못하는 유령 항목이 된다.
    /// ★이 규칙이 막는 범위는 좁다(과대 주장 금지)★: 같은 표식의 지각 신호 — 특히 `forget` **이후**에
    ///   도착하는 입력 에코 — 는 여기서 못 막는다. 그건 호출자(`OutputCore::emit`)가 finalize 플래그로
    ///   막는다(그쪽 주석이 그 인과의 정본). 주 방어선도 그쪽이다.
    /// ★표식이 다르면 **무조건 버린다** — 대소를 묻지 않는다★: 표식은 난수라 대소에 뜻이 없다
    ///   (`AgentProfile::epoch`). 자리를 바꾸는 권한은 [`Self::register`] 하나가 갖고, 그 등록이 산 화신의
    ///   첫 신호보다 먼저라는 것이 이 버리기를 안전하게 만든다(그 함수의 순서 문단).
    /// ★항목이 아예 없으면 받아 준다★: 등록을 거치지 않는 조립(하네스·직접 세운 `OutputCore`)도 관측이
    ///   되게 남긴 문이다. 운영에서는 `finish` 가 거둔 뒤의 지각 신호가 이 문으로 들어올 수 있지만, 그건
    ///   호출자의 finalize 재확인이 도로 거둔다(위 문단).
    ///
    /// ★같은 표식 안에서는 **seq 로 순서를 세운다**(load-bearing)★: emit 호출자가 둘이라(출력 pump ·
    ///   입력 에코를 낸 주입 스레드) 두 신호가 병행하면 **발행 순서와 적용 순서가 갈릴 수 있다** — 주입
    ///   스레드가 진행 신호를 seq N 으로 발행하고 잠시 멈춘 사이 pump 가 seq N+1 의 종료 신호를 적용해
    ///   버리면, 뒤늦게 깨어난 진행 신호가 **끝난 턴을 다시 진행 중으로** 되돌린다(그 수신자 앞 메일이
    ///   상한까지 다시 파킹된다). seq 는 replay 락 안에서 발급돼 출력의 정본 순서이므로(output_core.rs
    ///   emit), 더 낮은 seq 는 무시하면 그 역전이 닫힌다.
    /// ★seq 비교는 **같은 표식 안에서만** 유효하다★: 새 화신은 새 `OutputCore` 라 seq 가 0 부터 다시
    ///   시작한다 — 그래서 표식이 다르면 seq 를 아예 보지 않는다(그 항목은 [`Self::register`] 가 갈아치운다).
    /// ★그 대가(정직 표기)★: 시각은 락 밖에서 찍히므로 밀려난 신호가 **더 새 시각**을 들고 있는 게 보통이다.
    ///   즉 이 가드는 `last_signal` 을 아주 조금 **오래된 쪽**으로 유지한다 — 상한 판정이 그만큼 이르게
    ///   발화할 수 있다는 뜻이고, 폭은 스케줄 양자 하나라 30분 상한에서 무의미하다. 축의 정본을 **발행 순서**
    ///   로 잡은 결과다(적용 순서로 잡으면 끝난 턴이 되살아난다 — 위 문단).
    ///
    /// ★시각은 매 관측마다 갱신한다★: 정상적으로 길게 도는 턴은 신호가 계속 오므로 늙지 않고, 신호가
    ///   끊긴 것만 늙는다 — 소비자의 "멈춘 턴" 판정이 그 축 위에서 성립한다.
    /// ★남는 잔여와 그것을 지금 막지 않는 근거★: 두 화신의 표식이 2^-32 로 겹치면 이 일치 검사가 그
    ///   한 화신 동안 무력해진다(`ProfileRegistry::epoch_for_spawn` — 인접 보장은 같은 프로세스 안에서만
    ///   선다). 터졌을 때의 결말은 "턴 중인데 idle 로 보여 우편이 이르게 들어간다" 이고, 이 프로젝트는
    ///   그 이른 주입을 "안 가는 메일" 보다 낫다고 못박았다(ADR-0104).
    // ADR-0007
    pub fn observe_at(&self, id: AgentId, epoch: u32, seq: u64, signal: TurnSignal, at: Instant) {
        let mut g = self.lock();
        if let Some(cur) = g.get(&id) {
            if cur.epoch != epoch {
                return;
            }
            if seq < cur.last_seq {
                return;
            }
        }
        // ★`last_signal` 을 뒤로 되감지 않는다★: 시각은 락 **밖**에서 찍히므로(위 `observe`), 늦게 찍은
        //   신호가 먼저 적용될 수 있다. 되감기면 상한 판정이 그만큼 일찍 잔해로 오판한다 — 공짜로
        //   닫히므로 닫는다.
        let last_signal = match g.get(&id) {
            Some(cur) if cur.epoch == epoch => at.max(cur.last_signal),
            _ => at,
        };
        g.insert(
            id,
            Entry {
                epoch,
                in_turn: signal == TurnSignal::Progress,
                last_signal,
                last_seq: seq,
            },
        );
    }

    /// 이 (에이전트, epoch)의 관측값. `None` = 미관측(다른 epoch 만 관측된 경우 포함).
    pub fn get(&self, id: AgentId, epoch: u32) -> Option<TurnObservation> {
        let g = self.lock();
        g.get(&id)
            .filter(|e| e.epoch == epoch)
            .map(|e| TurnObservation {
                in_turn: e.in_turn,
                last_signal: e.last_signal,
            })
    }

    /// 관측상 턴 중인가. ★미관측은 false★ — "모른다" 를 여기서 참으로 바꾸지 않는다(사실 계층이
    /// 추측하면 소비자가 자기 폴백을 고를 수 없다).
    pub fn is_in_turn(&self, id: AgentId, epoch: u32) -> bool {
        self.get(id, epoch).is_some_and(|o| o.in_turn)
    }

    /// 지금 턴 중으로 관측된 전원. 소비자가 자기 상한으로 훑는 입구다.
    pub fn in_turn_snapshot(&self) -> Vec<(AgentId, u32, Instant)> {
        let g = self.lock();
        g.iter()
            .filter(|(_, e)| e.in_turn)
            .map(|(id, e)| (*id, e.epoch, e.last_signal))
            .collect()
    }

    /// 이 화신의 항목 제거. ★epoch 이 일치할 때만★ — 지각한 호출이 그 사이 새로 붙은 화신의 관측을
    /// 지우지 못하게 한다(reaper 의 sessions 제거 가드와 같은 원리).
    ///
    /// ★안 지우면 무슨 일이 나나★: 턴 중에 죽은 에이전트의 "턴 중" 표시가 남아, 그 이름 앞에서 기다리던
    ///   소비자(우편 파킹 등)가 영영 풀리지 않는다.
    /// ★청소의 주인은 `OutputCore::finish` 다★(그쪽 주석이 인과의 정본 — 종료 후 지각 emit 과의 경쟁까지
    ///   거기서 닫는다). 호출 지점은 **의도적으로 둘**이다: `finish`(주인)와 `emit` 의 finalize 재확인
    ///   경로(자기가 방금 넣은 것을 도로 거둔다 — 그 둘이 같은 표 뮤텍스를 타는 것이 경쟁을 닫는 논증
    ///   자체다). **세 번째 호출자를 늘리면** 그 인과가 갈라진다 — emit 쪽을 "금지된 두 번째 호출자"로
    ///   읽고 지우면 유령 항목 구멍이 다시 열린다.
    pub fn forget(&self, id: AgentId, epoch: u32) {
        let mut g = self.lock();
        if g.get(&id).is_some_and(|e| e.epoch == epoch) {
            g.remove(&id);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn unobserved_agent_has_no_entry() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        assert_eq!(t.get(id, 0), None);
        assert!(!t.is_in_turn(id, 0));
    }

    #[test]
    fn progress_then_end_flips_in_turn_and_keeps_the_entry() {
        // 소비자가 "마지막 신호가 언제였나" 를 계속 읽으므로 종료가 항목을 지우면 안 된다.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 0));
        t.observe(id, 0, 1, TurnSignal::Ended);
        assert!(!t.is_in_turn(id, 0));
        assert!(t.get(id, 0).is_some(), "종료 후에도 관측 사실은 남는다");
    }

    #[test]
    fn observation_is_keyed_by_epoch() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 0));
        assert_eq!(t.get(id, 1), None, "다른 epoch 은 미관측");
    }

    #[test]
    fn registration_replaces_the_entry_and_older_signals_are_dropped() {
        // 지각 신호가 옛 화신을 되살리면 그 항목은 아무도 못 지운다(종료 신호가 영영 안 온다).
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 0, 1, TurnSignal::Progress);
        t.register(id, 1);
        assert_eq!(t.get(id, 0), None, "등록이 옛 항목을 청소");
        assert_eq!(t.len(), 1, "재시작마다 죽은 항목이 쌓이지 않는다");
        assert!(!t.is_in_turn(id, 1), "등록만으로는 턴이 아니다");
        t.observe(id, 1, 1, TurnSignal::Progress);
        assert!(t.is_in_turn(id, 1));

        t.observe(id, 0, 1, TurnSignal::Progress);
        assert_eq!(t.get(id, 0), None, "옛 표식 지각 신호는 버린다");
        assert!(t.is_in_turn(id, 1), "현 화신 관측은 그대로");
    }

    // ★회귀 대상 — 표식이 난수가 된 뒤의 그 절반★: 옛 비교는 "현 표식이 더 크면 버린다" 였다. 죽어가는
    //   화신의 표식이 더 큰 순열에서는 그 가드가 통과해 지각 신호가 **산 화신의 항목을 덮었고**, 그 뒤
    //   산 화신의 조회는 미관측(=턴 아님)으로 답해 턴 중에 우편이 들어갔다. 그래서 이 테스트의 두 표식은
    //   **일부러 역전**돼 있다(산 화신 5 < 죽은 화신 900) — 대소 비교로 되돌리면 여기가 무너진다.
    #[test]
    fn a_late_signal_from_a_larger_tag_cannot_evict_the_live_incarnation() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let dying = 900;
        let live = 5;

        t.observe(id, dying, 3, TurnSignal::Progress);
        t.register(id, live);
        t.observe(id, live, 0, TurnSignal::Progress);
        assert!(t.is_in_turn(id, live), "전제: 산 화신이 턴 중으로 관측됨");

        // 배출이 덜 끝난 죽은 화신이 이제야 신호를 낸다.
        t.observe(id, dying, 4, TurnSignal::Ended);

        assert!(
            t.is_in_turn(id, live),
            "죽은 화신의 지각 신호가 산 화신의 관측을 덮으면 턴 중에 우편이 들어간다"
        );
        assert_eq!(
            t.get(id, dying),
            None,
            "죽은 화신의 항목은 되살아나지 않는다"
        );
        assert_eq!(t.len(), 1, "한 id 에 항목 하나");
    }

    #[test]
    fn last_signal_refreshes_on_every_observation() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(id, 0, 1, TurnSignal::Progress, t0);
        t.observe_at(
            id,
            0,
            1,
            TurnSignal::Progress,
            t0 + Duration::from_secs(20 * 60),
        );
        assert_eq!(
            t.get(id, 0).expect("관측됨").last_signal,
            t0 + Duration::from_secs(20 * 60),
            "갱신 없이 턴 시작 시각을 고정하면 정상 장기 턴이 소비자 상한에 잘린다"
        );
    }

    #[test]
    fn in_turn_snapshot_lists_only_agents_currently_in_a_turn() {
        let t = TurnObservations::new();
        let busy = AgentId::new_v4();
        let done_id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(busy, 3, 1, TurnSignal::Progress, t0);
        t.observe_at(done_id, 0, 1, TurnSignal::Ended, t0);
        assert_eq!(t.in_turn_snapshot(), vec![(busy, 3, t0)]);
    }

    #[test]
    fn a_signal_that_arrives_out_of_publication_order_is_ignored() {
        // ★순서 역전 회귀★: 진행 신호(seq N)가 종료 신호(seq N+1)보다 **늦게** 적용되면 끝난 턴이
        //   다시 진행 중으로 되돌아가고, 그 수신자 앞 메일이 상한까지 다시 파킹된다.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(id, 0, 5, TurnSignal::Progress, t0);
        t.observe_at(id, 0, 6, TurnSignal::Ended, t0);
        assert!(!t.is_in_turn(id, 0));

        // 뒤늦게 도착한 seq 5 진행 신호 — 무시돼야 한다.
        t.observe_at(id, 0, 5, TurnSignal::Progress, t0 + Duration::from_secs(1));
        assert!(
            !t.is_in_turn(id, 0),
            "늦게 적용된 옛 신호가 끝난 턴을 되살리면 안 된다"
        );
        assert_eq!(
            t.get(id, 0).expect("관측됨").last_signal,
            t0,
            "밀려난 신호는 상한 축을 다시 찍지 않는다"
        );

        // 가드가 정상 진행까지 막지는 않는다.
        t.observe_at(id, 0, 7, TurnSignal::Progress, t0 + Duration::from_secs(2));
        assert!(t.is_in_turn(id, 0));
    }

    #[test]
    fn an_accepted_signal_never_rewinds_last_signal() {
        // ★`at.max(cur.last_signal)` 의 유일한 방어★: 시각은 락 **밖**에서 찍히므로 늦게 찍은 신호가 먼저
        //   적용될 수 있다 — 그 경우에도 상한 축을 뒤로 되감으면 안 된다(되감으면 그만큼 이르게 잔해로
        //   오판한다). 위 seq 테스트는 **거부되는** 쪽만 훑으므로 이 줄을 지켜 주지 못한다.
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(1);

        // seq 5 가 늦은 시각을 들고 먼저 적용된다.
        t.observe_at(id, 0, 5, TurnSignal::Progress, later);
        // seq 6 은 정상 수용(더 최신 발행)이지만 시각은 더 이르다.
        t.observe_at(id, 0, 6, TurnSignal::Ended, t0);

        assert!(!t.is_in_turn(id, 0), "수용된 신호의 상태는 반영된다");
        assert_eq!(
            t.get(id, 0).expect("관측됨").last_signal,
            later,
            "수용 경로도 상한 축을 되감지 않는다(`at.max(cur.last_signal)`)"
        );
    }

    #[test]
    fn seq_comparison_does_not_leak_across_incarnations() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        let t0 = Instant::now();
        t.observe_at(id, 0, 900, TurnSignal::Progress, t0);
        t.register_at(id, 1, t0);
        t.observe_at(id, 1, 1, TurnSignal::Progress, t0);
        assert!(
            t.is_in_turn(id, 1),
            "새 화신의 낮은 seq 가 옛 화신의 높은 seq 때문에 버려지면 그 에이전트는 영구 미관측이 된다"
        );
    }

    // 등록 자체는 사실을 만들지 않는다 — 신호를 하나도 안 내는 백엔드가 상한 sweep 대상으로 올라오면
    // 죽을 때까지 60초마다 헛 도어벨이 울린다.
    #[test]
    fn registration_alone_is_not_a_turn() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.register(id, 7);
        assert!(!t.is_in_turn(id, 7));
        assert!(t.in_turn_snapshot().is_empty());
        t.forget(id, 7);
        assert_eq!(t.len(), 0, "등록분도 finish 가 거둔다");
    }

    #[test]
    fn forget_removes_only_the_matching_incarnation() {
        let t = TurnObservations::new();
        let id = AgentId::new_v4();
        t.observe(id, 1, 1, TurnSignal::Progress);
        // 지각 reap(옛 epoch) — 산 화신의 관측을 지우면 안 된다.
        t.forget(id, 0);
        assert!(t.is_in_turn(id, 1));
        t.forget(id, 1);
        assert_eq!(t.get(id, 1), None);
        assert_eq!(t.len(), 0);
    }
}
