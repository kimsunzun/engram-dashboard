//! busy — 수신자 턴 상태(busy/idle) 관측 + idle 게이트 seam(S18 메시징 v1 increment C2 · ADR-0104 결정 3).
//!
//! ★역할(C2 스코프)★: "수신자가 지금 턴 진행 중인가" 를 **관측된 사실로만** 들고 있어, MessagingService 가
//!   주입 전에 물어볼 수 있게 한다(spec §5 주입 타이밍 = idle 게이트 + 일괄 flush). 세 조각이다:
//!     ① `BusyGate` — 서비스가 소비하는 조회 seam(`is_busy(id, epoch)`). 운영 = `BusyTracker`.
//!     ② `BusyTracker` — (PeerId, epoch) 별 턴 상태 표 + tap 부착 관리.
//!     ③ `TurnProbe` — 호스트 어댑터가 턴 신호(진행/종료)를 넣는 수신구. 진행 신호는 표를 갱신하고,
//!        턴 종료 신호는 표를 지운 뒤 flush 트리거를 통지한다(`IdleNotifier`).
//!   request/reply(C3)·그룹(C4)은 범위 밖.
//!
//! ★positive-knowledge-only(load-bearing — spec §5 capability 폴백)★: 표에 **없는** (id, epoch) 는 전부
//!   **idle 취급**이다(= 즉시 주입). "모른다" 를 busy 로 해석하면 관측 불가 백엔드·tap 미부착 창(부팅 초기,
//!   attach 실패)에서 배달이 **영구 대기**한다(ADR-0104 영향/불변식: "관측 불가 백엔드에서 idle 게이트를
//!   강제하면 배달이 영구 대기"). 그래서 busy 는 **관측된 사실이 있을 때만** 참이다 — 자료구조도 그 의미를
//!   구조적으로 못 박는다: 표는 `HashSet<(PeerId, epoch)>` 로 **busy 인 것만** 담고, 부재 = idle ∨ 미관측
//!   (둘을 구분하지 않는다 — 둘 다 즉시 주입이 정답이라 구분할 이유가 없다).
//!
//! ★busy 관측 capability = `output.structured` 프록시(내부 선택 — 보고 대상)★: 턴 이벤트(MessageDone)는
//!   백엔드 decoder 가 있는 에이전트에서만 나온다(claude stream-json). 그리고 decoder 는 `structured`
//!   출력 capability 와 **정확히 같은 조건**으로 존재한다(json 모드 = decoder 있음 = structured=true /
//!   터미널 모드 = TerminalBytes 만). 그래서 C2 는 core/protocol 에 새 capability 필드(`output.busy` 같은)를
//!   추가하지 않고 **`structured` 를 busy-관측 가능성의 프록시로** 쓴다 — 그 프록시 값은
//!   `LiveAgent::turn_signal` 로 로스터 항목에 실려 오고(ADR-0116 결정 7), tap 부착 대상도 같은 조건이라
//!   게이트가 보는 집합과 tap 이 관측하는 집합이 **구조적으로 일치**한다.
//!   ★4차 개정(ADR-0116)★: 비-structured 세션은 **이제 로스터에 있다**(멤버십 조건이 아니다) — 다만 발송
//!   경로가 그 부류에 대해 이 게이트를 **아예 묻지 않고** 즉시 주입한다(`turn_signal == false`). 즉 이
//!   모듈이 보는 집합은 여전히 "턴 이벤트가 나오는 에이전트" 로 유지된다. 프록시가 깨지는 날(= structured
//!   이지만 턴 이벤트가 없는 백엔드 등장) 그때 진짜 capability 필드를 추가한다 — ADR-0104 capability
//!   원칙과 정합(관측 불가 = 즉시 주입 폴백).
//!
//! ★콜백 규율(load-bearing — 절대 위반 금지)★: `TurnProbe::on_progress`/`on_turn_done` 은 **호스트의
//!   출력 pump 스레드**가 부르는 동기 콜백이다. 여기서 하는 일은 ① 작은 락 구간의 HashSet 갱신 ② 논블록 채널 send
//!   **둘뿐**이다 — 주입(inject)·manager 호출·messaging 락 취득·blocking write 를 **하지 않는다**. 이걸
//!   어기면 출력 pump 가 배달 작업 뒤에서 막혀 전 에이전트의 출력 스트림이 지연된다(ws.rs finding 5 와
//!   같은 계열의 사고). 실제 flush 는 통지를 받은 flush worker 가 수행한다.
//!
//! ★tap 은 live-only — replay 부트스트랩은 **거부된 설계**다(load-bearing, C2 리뷰 fix 1)★: 초기 C2 는
//!   `manager.subscribe`(구독 시점 링버퍼 과거 전체를 replay)를 써서 "과거를 먹으면 상태가 대략 현재로
//!   수렴한다" 는 공짜 부트스트랩을 노렸다. **폐기한다.** 이유: resume 스폰은 transcript 를 링에 seed 하고
//!   (core manager.rs ADR-0079 seed-before-publish), 그 transcript 가 **턴 중간에 끊긴** 것이면(killed
//!   incarnation) TextDelta/Structured 로 끝나고 **MessageDone 이 없다** → tap 이 그 과거를 먹으면
//!   (id, new_epoch) 를 busy 로 찍는데 그 턴의 종료 통지는 **영원히 오지 않는다**(이미 지나간 기록이므로).
//!   결과 = 깨울 수 없는 false-busy → 그 수신자 앞 모든 발송이 TTL 만료까지 파킹된다(배달이 안 가는 것 =
//!   메시징 최악 실패 모드). 그래서 tap 은 **구독 이후 발생분만** 받는다(호스트 어댑터
//!   `messaging_host::ManagerTapHost::subscribe_output` 의 `after_seq = u64::MAX` 주석 — 그 규율은
//!   `TapHost` 계약의 일부다). 부트스트랩 없이 시작하는 대가는 positive-knowledge-
//!   only 폴백이 이미 흡수한다 — 관측 전엔 idle = 즉시 주입(늦게 가는 것보다 안 가는 것이 나쁘다).
//!
//! ★busy 상한(fail-open 안전 밸브 — round-3 finding 4)★: MessageDone 은 **유일한 in-band 해제**다. 턴이
//!   비정상 종료(파싱 실패·decoder 이상·`result` 라인 누락)하면 busy 표시가 **영구히** 남아 그 수신자 앞
//!   모든 배달이 도어벨마다 접히고 결국 TTL 로 만료된다("안 가는 것" = 메시징 최악 실패 모드). 그래서 표는
//!   busy 로 **관측된 시각**을 함께 들고, `BUSY_MAX_TURN`(아래) 을 넘긴 항목은 주기 sweep(lib.rs 60s)이
//!   청소하고 그 id 를 **도어벨로 깨운다**(대기 메일이 그때 배달된다). 발생 빈도는 미측정(spec §7 항목).
//!
//! ★알려진 미확인(측정 항목 — spec §7)★: 중첩 Task 서브에이전트의 `result` 라인이 **부모 턴 종료**
//!   신호로 새는지 미검증이다. 새면 부모가 아직 턴 중인데 idle 로 오판해 조기 주입할 수 있다
//!   (유실은 없고 타이밍만 어긋남). C2 는 이를 **해결하지 않고 실 하네스 측정 항목으로 남긴다**.
//!
//! 워크스페이스 crate import 0(ADR-0110 — 컴파일러 강제).
// ADR-0103
// ADR-0104
// ADR-0110

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::PeerId;

/// ★busy 상한(fail-open 안전 밸브 — round-3 finding 4, 내부 선택: **30분**)★: 마지막 턴 진행 관측으로부터
///   이 시간을 넘긴 busy 표시는 주기 sweep 이 **비정상 종료된 턴의 잔해**로 보고 청소한다(모듈 헤더).
///
/// ★왜 상한이 안전한가(무엇을 잃나)★: 오판(실제로는 턴 중인데 idle 로 봄)의 결과는 **턴 중 주입**이고,
///   claude CLI 는 턴 중 stdin 을 내부 큐에 넣어 **다음 턴에 읽는다**(실측된 사용자 대면 동작 — spec §7).
///   즉 최악의 대가는 "언제 읽히나" 가 흐려지는 것뿐이며 **유실은 없다**. 반면 상한이 없으면 그 수신자 앞
///   메일이 TTL 까지 전부 안 간다 — 비교 불가하게 나쁘다(ADR-0104 "늦게 가는 것 < 안 가는 것").
/// ★왜 30분인가★: 사람 대화 수준 메시지율에서 30분 무-출력 턴은 정상 범위를 크게 벗어난다(도구 호출·
///   delta·usage 중 하나라도 오면 아래 `mark_busy` 가 시각을 갱신하므로, 30분은 "출력이 완전히 멈춘 채
///   MessageDone 도 없는" 구간을 뜻한다). 더 짧으면 정상 장기 턴을 자르고, 더 길면 회복이 늦다.
/// ★시각은 **마지막 관측**으로 갱신한다(턴 시작 고정이 아니다)★: 갱신하면 정상적으로 길게 도는 턴은
///   계속 busy 로 유지되고(오판 없음), 관측이 끊긴 것만 늙는다 — 상한의 목적(잔해 청소)에 정확히 맞는다.
pub const BUSY_MAX_TURN: Duration = Duration::from_secs(30 * 60);

/// ★idle 게이트 조회 seam(ADR-0012)★ — MessagingService 가 "이 수신자가 지금 턴 중인가" 를 묻는 유일한 문.
///
/// ★왜 별도 trait 인가(DeliveryPort 에 합치지 않는 이유)★: `DeliveryPort` 는 **배달**(inject/roster/이름)
///   전용 seam 이다. 턴 상태 관측은 출처(출력 스트림 tap)와 수명(구독)이 배달과 완전히 다르므로 섞으면
///   두 축이 함께 커진다 — 헤드리스 단위 테스트도 "가짜 배달 + 가짜 busy" 를 **독립으로** 조립해야 조합
///   폭발을 피한다. 그래서 게이트는 이 좁은 trait 하나로 분리한다.
/// ★계약★: 순수 조회 — 부작용 없음, 블로킹 없음(짧은 락만). messaging 락을 **든 채** 불려도 안전해야
///   한다(현 호출부는 락 밖에서 부르지만, 이 계약을 지켜 두면 미래 호출 지점이 늘어도 데드락이 없다).
pub trait BusyGate: Send + Sync {
    /// 이 (id, epoch) 가 **관측상** 턴 진행 중인가. ★모르는 대상은 반드시 false(idle)★ —
    ///   positive-knowledge-only(모듈 헤더). true 는 "턴 중이라는 관측 근거가 있다" 는 뜻이다.
    fn is_busy(&self, id: PeerId, epoch: u32) -> bool;
}

/// 게이트 미배선/관측 불가 폴백 — **항상 idle**(= 즉시 주입, spec §5 capability 폴백).
///
/// ★왜 필요한가★: 게이트가 없는 조립(실험 bin·게이트를 검증하지 않는 단위 테스트)에서 서비스가 C1 과
///   **byte-identical** 하게 동작하게 하는 기본값이다. "게이트를 안 꽂았다" 가 "배달이 멈춘다" 로 번지지
///   않게 하는 안전 기본값(fail-open) — 메시징의 실패 모드는 "늦게 가는 것" 보다 "안 가는 것" 이 훨씬 나쁘다.
pub struct AlwaysIdleGate;

impl BusyGate for AlwaysIdleGate {
    fn is_busy(&self, _id: PeerId, _epoch: u32) -> bool {
        false
    }
}

/// ★tap 부착 seam(ADR-0012)★ — 출력 스트림 구독을 트레잇 뒤로 밀어, 단위 테스트가 실 PTY·claude 없이
///   부착 정책(중복 방지·epoch 검증·실패 재시도)을 단언하게 한다. 운영 구현은 호스트 소유
///   (데몬 `messaging_host::ManagerTapHost` — ADR-0110 결정 3).
///
/// ★비용 경고(호출 지점 제약 — load-bearing)★: 구현(운영)은 호스트 출력 코어의 subscribers 락을 잡고 들어간다.
///   그래서 이 메서드는 **status 콜백 같은 짧아야 하는 경로에서 부르면 안 된다** — flush worker 의
///   blocking pool 에서만 부른다(ws.rs). live-only 구독이라 링 replay 비용은 없지만(아래) core 락 구간에
///   들어가는 호출이라는 성질은 그대로다.
pub trait TapHost: Send + Sync {
    /// 이 참여자의 턴 관측을 시작한다 — 호스트의 출력 스트림에 이 수신구를 배선한다. ★live-only★
    ///   (배선 이후 발생분만 — 과거 replay 금지, 모듈 헤더 "replay 부트스트랩 거부").
    ///
    /// ★`probe` 는 무엇인가(ADR-0110 결정 4)★: 출력 이벤트→턴 신호 **분류**는 백엔드 지식(claude
    ///   stream-json 등)이라 이 커널이 알지 않는다. 호스트 어댑터가 자기 출력 계층에 붙어 분류한 뒤 이
    ///   수신구의 `on_progress`/`on_turn_done` 를 부른다. 커널은 신호 어휘만 소유한다.
    ///
    /// ★`expect_epoch` 는 장식이 아니다(round-3 finding 5 — 유령 tap 누수 차단)★: 구현은 **배선 지점에서**
    ///   그 id 의 현재 epoch 이 `expect_epoch` 인지 확인해야 하고, 아니면 `StaleEpoch` 를 돌려주며 **구독을
    ///   남기지 않아야** 한다(이미 붙였다면 되돌린다). 왜: `attach` 의 사전 검증과 여기 배선 사이에 재시작이
    ///   끼면 수신구가 **새 epoch 의 출력 코어** 에 붙는데 부착 표시는 `(id, 옛 epoch)` 로 남는다 → 그 tap 이
    ///   보는 신호는 전부 양성 게이트에서 버려지고(관측 0), 뒤이어 새 epoch 용 정상 attach 가 수신구를
    ///   **하나 더** 붙인다(tap 은 명시 해제를 하지 않으므로 그 유령은 그 코어 수명 동안 남아 통지를 중복시킨다).
    fn subscribe_output(
        &self,
        id: PeerId,
        expect_epoch: u32,
        probe: Arc<TurnProbe>,
    ) -> Result<(), SubscribeError>;

    /// 이 id 의 **현재** epoch(로스터 기준). 부재(reap 완료)면 None.
    ///
    /// ★왜 필요한가(fix 6 — epoch 검증 attach)★: Attach 요청은 로스터 diff 시점 epoch 을 실어 채널을 타고
    ///   오므로, worker 가 집행할 때 이미 stale 일 수 있다(그 사이 재시작 = epoch bump). 검증 없이 붙이면
    ///   **이미 사라진 epoch** 키로 부착/busy 표를 만들어(그 core 는 곧 drop) 아무도 지우지 않는 유령 항목이
    ///   남고, 같은 id 에 tap 이 둘 붙어 통지가 중복된다. 그래서 attach 는 집행 직전 현재 epoch 을 확인한다.
    fn current_epoch(&self, id: PeerId) -> Option<u32>;
}

/// tap 구독 실패 사유 — 호출자(`BusyTracker::attach`)가 **재시도할 값어치가 있는 실패**와 **아예 대상이
///   아닌 것**을 구분하려고 나눈다(round-3 finding 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeError {
    /// 구독 지점에서 확인한 현재 epoch 이 기대와 다름(그 사이 재시작/reap) — 구독은 남지 않았다.
    ///   현재 epoch 용 Attach 가 로스터 diff 로 뒤따라 오므로 호출자는 아무것도 하지 않는다.
    StaleEpoch { current: Option<u32> },
    /// 구독 자체 실패(부재/죽음/transport 오류) — 호출자가 재시도 경로를 열어야 한다.
    Failed(String),
}

/// ★턴 종료(idle 전이) 통지 seam★ — 턴 관측(`TurnProbe`)이 "이 에이전트 턴이 끝났다" 를 알리는 출구. 운영 구현은
///   flush worker 채널로 논블록 send 한다(ws.rs `ChannelIdleNotifier`), 단위 테스트는 기록만 한다.
///
/// ★계약(load-bearing)★: **논블록·비-재진입**이어야 한다 — pump 스레드 콜백에서 불린다(모듈 헤더 콜백
///   규율). 락 취득·IO·주입 금지.
/// ★idempotency 의존(수용된 설계)★: 통지는 **MessageDone 마다** 나간다(전이 여부를 따지지 않는다). 왜:
///   "직전이 busy 였을 때만" 으로 좁히면, 어시스턴트 이벤트 없이 곧장 `result` 로 끝나는 턴에서 통지가
///   빠져 파킹이 다음 턴까지 stranded 된다 — 배달 누락(치명)보다 **잉여 통지(무해)** 를 택한다. 잉여
///   통지의 대가는 빈 큐 drain no-op 뿐이다(flush 경로가 execution 시점 재검증 + 빈 큐 조기 반환으로
///   idempotent — service.rs `flush_for` / `flush_for_agent`). 채널 압력은 **id 별 coalescing** 이
///   유계로 묶는다(ws.rs `IdleCoalescer` — 같은 id 의 미처리 Idle 이 이미 큐에 있으면 enqueue 를 접는다).
pub trait IdleNotifier: Send + Sync {
    /// 이 에이전트가 방금 턴을 끝냈다(= 쌓인 파킹을 일괄 주입할 시점). **논블록**.
    fn notify_idle(&self, id: PeerId);
}

/// tap(쓰기 측)과 tracker/서비스(읽기 측)가 공유하는 턴 상태 — **부착 표시**와 **busy 표시** 두 표다.
///
/// ★왜 Arc 로 쪼갰나★: tap sink 는 core 의 구독자 목록이 소유하므로 tracker 보다 오래(또는 짧게) 살 수
///   있다. 상태를 `Arc<TurnStates>` 로 분리하면 tap 이 tracker 자체를 잡지 않아 소유 그래프가 단순해진다
///   (tracker→상태, tap→상태 — 순환 없음).
/// ★왜 `attached` 가 tracker 가 아니라 여기 있나(fix 9)★: tap 이 busy 를 찍기 전에 "나는 아직 유효한
///   부착인가" 를 물어야 하기 때문이다(아래 mark_busy 양성 게이트). tracker 에 두면 tap 이 tracker 를
///   붙잡아야 해 소유 그래프가 순환한다.
/// ★락 순서(load-bearing — ADR-0006)★: **attached → busy** 한 방향뿐이다(역순 금지). 외부 호출
///   (`host.subscribe_output`)은 **두 락을 모두 놓은 상태**에서만 한다.
struct TurnStates {
    /// ★tap 부착이 확정된 (PeerId, epoch)★ — 두 역할을 한 표가 겸한다:
    ///   ① 중복 subscribe 방지(같은 스트림에 tap 이 여러 개면 상태 갱신·통지가 N배).
    ///   ② `mark_busy` 의 **양성 게이트**(fix 9 — 아래).
    attached: Mutex<HashSet<(PeerId, u32)>>,
    /// ★턴 진행 중으로 관측된 (PeerId, epoch) → **마지막 관측 시각**★. 부재 = idle ∨ 미관측(= 즉시 주입,
    ///   모듈 헤더). 값(시각)은 상한 sweep(`BUSY_MAX_TURN`)이 "비정상 종료된 턴의 잔해" 를 가려내는 축이다.
    busy: Mutex<HashMap<(PeerId, u32), Instant>>,
    /// 턴 종료 통지 출구(논블록 계약).
    notifier: Arc<dyn IdleNotifier>,
}

impl TurnStates {
    /// 턴 시작/진행 관측 — **현재 부착으로 등록된 키에 대해서만** busy 로 표시한다.
    ///
    /// ★양성 attach 게이트(load-bearing — fix 9)★: 왜 무조건 insert 가 아닌가. epoch 교체 직후, **옛
    ///   incarnation 의 core** 가 아직 큐에 남은 이벤트를 배출하는 창이 있다(pump 가 EOF 로 끝나기 전).
    ///   그 옛 tap 의 `mark_busy` 를 무조건 받으면 attach 가 방금 청소한 `(id, 옛 epoch)` 항목이 **되살아나고**,
    ///   그 epoch 의 tap 은 이미 사라질 운명이라 MessageDone 이 오지 않으며 Detach 도 그 id 전체를 지운 뒤라
    ///   아무도 지우지 않는다 → 지워지지 않는 유령 busy. 부착 표에 있는 키만 받으면 이게 구조적으로 막힌다
    ///   (부착 해제 = 그 tap 의 관측 권한 만료).
    /// ★시각은 매 관측마다 갱신한다(load-bearing — `BUSY_MAX_TURN` 주석)★: 정상적으로 오래 도는 턴은
    ///   delta/도구 호출이 계속 오므로 늙지 않고, 관측이 끊긴 잔해만 상한에 걸린다.
    /// ★`Instant::now()` 를 여기서 부르는 건 의도적 예외★: 이 모듈은 pump 콜백 층이라 messaging 의 순수성
    ///   불변식(clock injection) 대상이 아니다(crate 헤더 lib.rs 의 예외 구역). 판정(sweep)은 주입된 now 를
    ///   받으므로 결정적 테스트는 유지된다 — 시계를 읽는 곳은 이 한 지점뿐이다.
    fn mark_busy(&self, key: (PeerId, u32)) {
        self.mark_busy_at(key, Instant::now());
    }

    /// 시각 주입형(테스트가 상한 경계를 결정적으로 구동한다 — 운영 경로는 위 `mark_busy`).
    fn mark_busy_at(&self, key: (PeerId, u32), at: Instant) {
        // 락 순서 attached → busy(모듈 주석). attached 를 든 채 busy 를 잡는다 — 그 사이 attach 가 바뀌어
        //   유령 항목이 생기는 창을 없애려면 두 표를 한 임계구역에서 봐야 한다.
        let atk = self.attached.lock().expect("busy attached poisoned");
        if !atk.contains(&key) {
            return;
        }
        self.busy
            .lock()
            .expect("busy states poisoned")
            .insert(key, at);
    }

    /// 턴 종료 관측 — busy 해제 후 **락을 놓고** 통지한다(락 보유 중 외부 호출 금지 — ADR-0006 정신).
    ///
    /// ★해제·통지는 attach 게이트를 걸지 않는다(의도적)★: 제거는 언제나 안전하고(없는 키 remove = no-op),
    ///   통지는 잉여여도 빈 큐 no-op 이다. 반대로 게이트를 걸면 "부착 표가 방금 갱신됐다" 는 이유로 종료
    ///   통지가 삼켜져 파킹이 stranded 될 수 있다 — 누락 < 잉여(IdleNotifier 주석).
    fn mark_idle(&self, key: (PeerId, u32)) {
        {
            let mut g = self.busy.lock().expect("busy states poisoned");
            g.remove(&key);
        }
        // 전이 여부와 무관하게 매번 통지(위 IdleNotifier idempotency 주석 — 누락 < 잉여).
        self.notifier.notify_idle(key.0);
    }

    fn is_busy(&self, key: (PeerId, u32)) -> bool {
        self.busy
            .lock()
            .expect("busy states poisoned")
            .contains_key(&key)
    }

    /// ★상한 초과 busy 잔해 청소(round-3 finding 4)★ — 마지막 관측이 `BUSY_MAX_TURN` 이전인 항목을 제거하고
    ///   그 **PeerId 목록**(중복 제거)을 돌려준다. 호출자가 락을 놓은 뒤 그 id 들을 도어벨로 깨운다.
    ///
    /// ★왜 통지를 여기서 하지 않나(ADR-0006)★: `notifier` 는 외부 호출이다 — busy 락을 든 채 부르면 락
    ///   보유 중 외부 호출 금지 규율을 깬다. 그래서 이 함수는 **순수 제거 + 목록 반환**만 하고 통지는
    ///   `BusyTracker::sweep_stale_busy` 가 락 밖에서 한다.
    fn sweep_stale(&self, now: Instant, max: Duration) -> Vec<PeerId> {
        let mut woken: Vec<PeerId> = Vec::new();
        let mut g = self.busy.lock().expect("busy states poisoned");
        g.retain(|(id, _epoch), marked| {
            let stale = now.saturating_duration_since(*marked) >= max;
            if stale && !woken.contains(id) {
                woken.push(*id);
            }
            !stale
        });
        woken
    }

    /// 부착 표시 선점 — 새로 표시했으면 true, 이미 있었으면 false(중복 subscribe 금지).
    /// 새로 표시할 때 같은 id 의 **다른 epoch** 표시는 청소한다(한 id 에 살아있는 epoch 은 하나 — ADR-0007).
    fn claim_attached(&self, key: (PeerId, u32)) -> bool {
        let mut at = self.attached.lock().expect("busy attached poisoned");
        if !at.insert(key) {
            return false;
        }
        at.retain(|(k, e)| *k != key.0 || *e == key.1);
        true
    }

    /// 부착 표시 1건 해제(attach 실패·패닉 롤백 — `AttachGuard`).
    fn release_attached(&self, key: (PeerId, u32)) {
        self.attached
            .lock()
            .expect("busy attached poisoned")
            .remove(&key);
    }

    #[cfg(any(test, feature = "test-harness"))]
    fn attached_len(&self) -> usize {
        self.attached.lock().expect("busy attached poisoned").len()
    }

    /// 이 id 의 모든 상태 제거(로스터 이탈 = 죽음). 죽은 에이전트의 busy 플래그가 남아 그 이름 앞
    ///   파킹이 영영 대기하는 걸 막는다(stale-flag 청소) + 다음 등장에 재부착되게 부착 표시도 지운다.
    fn forget(&self, id: PeerId) {
        // 락 순서 attached → busy.
        self.attached
            .lock()
            .expect("busy attached poisoned")
            .retain(|(k, _)| *k != id);
        self.busy
            .lock()
            .expect("busy states poisoned")
            .retain(|(k, _), _| *k != id);
    }

    /// 이 id 의 **다른 epoch** busy 표시만 제거(현 epoch 은 보존). epoch 교체(재시작/재활성화)는 같은
    ///   PeerId 의 맵 항목을 바꾸므로(ADR-0007) 한 id 에 살아있는 epoch 은 **항상 하나**다 — 옛 epoch 의
    ///   busy 표시는 그 순간 무의미해진다. 안 지우면 재시작마다 죽은 항목이 한 개씩 누적된다.
    fn forget_other_epochs(&self, id: PeerId, keep: u32) {
        self.busy
            .lock()
            .expect("busy states poisoned")
            .retain(|(k, e), _| *k != id || *e == keep);
    }
}

/// ★부착 표시 롤백 가드(fix 8b)★ — subscribe 가 `Err` 를 내든 **패닉으로 unwind** 하든 부착 표시를
///   되돌린다. 왜 Ok/Err 분기 롤백으로 부족한가: subscribe 는 core 락 구간에 들어가고 그 안에서 다른
///   구독자 sink 를 호출할 수도 있어(패닉 전파 가능) Err 아닌 unwind 가 현실적으로 가능하다. 그때 표시가
///   남으면 그 (id, epoch) 는 **영영 tap 없이** 돌고(재부착 요청이 중복으로 접힌다) 양성 게이트 때문에
///   busy 도 못 찍혀 "게이트가 조용히 사라진" 상태가 된다 — 조용한 기능 상실은 실패보다 나쁘다.
struct AttachGuard {
    shared: Arc<TurnStates>,
    key: (PeerId, u32),
    armed: bool,
}

impl AttachGuard {
    fn new(shared: Arc<TurnStates>, key: (PeerId, u32)) -> Self {
        Self {
            shared,
            key,
            armed: true,
        }
    }
    /// 부착 확정 — 이제 표시를 유지한다.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        if self.armed {
            self.shared.release_attached(self.key);
        }
    }
}

/// `BusyTracker::attach` 집행 결과 — 호출자(flush worker)가 실패를 **피드백**할 수 있게 결과를 돌려준다.
///
/// ★왜 결과가 필요한가(fix 8a)★: attach 가 실패를 조용히 삼키면 그 에이전트는 tap 없이 남고, 로스터
///   diff 는 이미 "부착됨" 으로 스냅샷을 갱신했으므로 **다음 diff 가 재시도하지 않는다**(스냅샷이 같으니
///   Attach 를 다시 내지 않는다). 그래서 worker 가 실패를 받아 diff 스냅샷을 무효화해야 한다(ws.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachOutcome {
    /// 새로 부착 성공.
    Attached,
    /// 이미 그 (id, epoch) 에 붙어 있음 — 중복 요청을 접었다(정상).
    AlreadyAttached,
    /// 부착 대상이 아님 — 로스터에서 사라졌거나 큐잉 epoch 이 더 이상 현재가 아니다(fix 6).
    ///   현재 epoch 용 Attach 가 로스터 diff 에서 뒤따라 오므로 호출자는 아무것도 하지 않는다.
    Stale,
    /// subscribe 실패(부재/죽음) — 부착 표시는 롤백됐다. 호출자가 diff 스냅샷을 무효화해 재시도를 열어야 한다.
    Failed,
}

/// ★BusyTracker — 턴 상태 표 + tap 부착 관리(C2)★. 데몬 부팅에서 하나 만들어 셋이 공유한다:
///   - MessagingService(게이트 조회 — `BusyGate`),
///   - flush worker(Attach/Detach 집행),
///   - (간접) 각 에이전트에 배선된 `TurnProbe`(상태 갱신).
///
/// ★락 규율(load-bearing)★: 두 표의 순서는 **attached → busy** 한 방향뿐이다(`TurnStates` 주석).
///   외부 호출(`host.subscribe_output`)은 **두 락을 모두 놓은 상태**에서만 한다 — subscribe 는 내부에서
///   호스트 출력 코어의 subscribers 락을 잡고, 그 구간에서 우리 수신구가 호출될 수 있으므로(live emit 과 직렬화) 락을
///   든 채 부르면 자기 재진입 데드락이다. 이게 attach 가 "표시 → 락 해제 → subscribe → (실패/패닉 시)
///   가드 롤백" 순서인 이유다.
pub struct BusyTracker {
    shared: Arc<TurnStates>,
    host: Arc<dyn TapHost>,
}

impl BusyTracker {
    pub fn new(host: Arc<dyn TapHost>, notifier: Arc<dyn IdleNotifier>) -> Self {
        Self {
            shared: Arc::new(TurnStates {
                attached: Mutex::new(HashSet::new()),
                busy: Mutex::new(HashMap::new()),
                notifier,
            }),
            host,
        }
    }

    /// ★tap 부착(C2)★ — 이 (id, epoch) 의 출력 스트림에 턴 관측 tap 을 붙인다. 로스터 diff 가 새로 등장/
    ///   epoch bump 한 **모든 id** 에 대해 flush worker 를 통해 부른다(이름 유일성과 무관 — tap 은 id 단위).
    ///
    /// 순서(각 단계가 load-bearing):
    ///   1. **epoch 검증(fix 6)** — 큐잉된 epoch 이 아직 현재인가(`TapHost::current_epoch`). 아니면 `Stale`
    ///      로 즉시 반환(그 새 epoch 용 Attach 가 뒤따라 온다). ★이건 **싼 조기 차단**일 뿐이다★ — 이
    ///      확인과 3번 구독 사이에도 재시작 창이 있어서, **구독 지점의 재확인**(TapHost 계약 · round-3
    ///      finding 5)이 실제 방어선이다. 그쪽이 `StaleEpoch` 를 내면 여기서도 `Stale` 로 접는다.
    ///   2. **부착 표시 선점** — 중복 요청 접기 + 옛 epoch 표시 청소(짧은 락).
    ///   3. **락 해제 후 subscribe** — core 락 구간에 들어가므로 우리 락을 들고 부르면 재진입 데드락.
    ///   4. **실패/패닉 롤백** — `AttachGuard`(Drop)가 되돌린다(fix 8b).
    ///
    /// ★반드시 flush worker(blocking pool)에서만★: subscribe 가 core subscribers 락 구간에 들어가므로
    ///   status 콜백에서 부르면 로스터 이벤트 forwarding 이 그만큼 막힌다(ws.rs finding 5 계열).
    /// ★실패가 배달을 막지는 않는다★: 부착 실패 대상은 게이트가 모르므로 idle = 즉시 주입 폴백
    ///   (positive-knowledge-only). 대신 `Failed` 를 돌려 호출자가 **재시도 경로를 열게** 한다(fix 8a).
    pub fn attach(&self, id: PeerId, epoch: u32) -> AttachOutcome {
        // 1) epoch 현재성 검증(fix 6) — stale Attach 로 유령 표를 만들지 않는다.
        match self.host.current_epoch(id) {
            Some(cur) if cur == epoch => {}
            other => {
                tracing::debug!(
                    agent = %id,
                    queued_epoch = epoch,
                    current_epoch = ?other,
                    "턴 tap attach skip: 큐잉 epoch 이 현재가 아님(부재 또는 재시작) — 현재 epoch 용 Attach 를 기다린다"
                );
                return AttachOutcome::Stale;
            }
        }
        // 2) 부착 표시 선점(중복 접기 + 옛 epoch 표시 청소).
        if !self.shared.claim_attached((id, epoch)) {
            return AttachOutcome::AlreadyAttached;
        }
        // 3) ★락 해제 상태에서만 subscribe★ + 패닉/실패 롤백 가드 무장.
        let mut guard = AttachGuard::new(self.shared.clone(), (id, epoch));
        // 옛 incarnation 의 busy 표시 청소(그 epoch 은 더 이상 존재하지 않는다).
        self.shared.forget_other_epochs(id, epoch);
        let probe = self.make_probe();
        match self.host.subscribe_output(id, epoch, probe) {
            Ok(()) => {
                guard.disarm();
                AttachOutcome::Attached
            }
            // ★구독 지점에서 잡힌 stale(round-3 finding 5)★: 유령 구독은 host 가 회수했고 부착 표시는
            //   가드가 되돌린다. 재시도하지 않는다 — 현재 epoch 용 Attach 가 로스터 diff 로 뒤따라 온다
            //   (여기서 재시도하면 같은 stale 판정을 한 번 더 하는 낭비다).
            Err(SubscribeError::StaleEpoch { current }) => {
                tracing::debug!(
                    agent = %id,
                    queued_epoch = epoch,
                    current_epoch = ?current,
                    "턴 tap attach skip: 구독 지점에서 epoch 교체 관측 — 현재 epoch 용 Attach 를 기다린다"
                );
                AttachOutcome::Stale
            }
            Err(SubscribeError::Failed(e)) => {
                // 가드가 drop 되며 부착 표시를 되돌린다(재시도 가능 상태).
                tracing::debug!(
                    agent = %id,
                    epoch,
                    "턴 tap 부착 실패(이미 죽음/부재) — 부착 표시 롤백, 그 전까지 idle 폴백: {e}"
                );
                AttachOutcome::Failed
            }
        }
    }

    /// ★busy 상한 sweep(round-3 finding 4)★ — `BUSY_MAX_TURN` 을 넘긴 busy 잔해를 청소하고, 그 id 들을
    ///   **도어벨로 깨운다**(대기 중인 파킹이 그때 배달된다). lib.rs 의 60s sweep task 가 부른다.
    ///
    /// ★깨우기가 fix 의 절반이다★: 표만 지우면 그 수신자 앞 파킹은 **다음 트리거**(등장/턴 종료/새 발송)
    ///   까지 그대로 앉아 있다 — 그런데 애초에 이 상황은 "그 트리거가 오지 않는" 상황이다(턴 이벤트 유실).
    ///   그래서 청소한 id 마다 idle 통지를 내 flush 를 유도한다.
    /// ★락 밖 통지(ADR-0006)★: 제거(락)와 통지(락 밖)를 분리한다 — `TurnStates::sweep_stale` 주석.
    /// 반환 = 깨운 id 수(관측/테스트용).
    pub fn sweep_stale_busy(&self, now: Instant) -> usize {
        let woken = self.shared.sweep_stale(now, BUSY_MAX_TURN);
        for id in &woken {
            tracing::warn!(
                agent = %id,
                max_secs = BUSY_MAX_TURN.as_secs(),
                "busy 상한 초과 — 턴 종료 관측 없이 늙은 busy 표시 청소 후 flush 도어벨(fail-open 안전 밸브)"
            );
            self.shared.notifier.notify_idle(*id);
        }
        woken.len()
    }

    /// 로스터 이탈(죽음) 처리 — 그 id 의 턴 상태·부착 표시를 지운다.
    ///
    /// ★unsubscribe 를 부르지 않는 이유★: 세션이 reap 되면 그 `OutputCore` 와 구독자 목록이 통째로 drop
    ///   되므로 tap 은 자동 회수된다(코어가 send 실패 sink 를 스스로 제거하는 경로도 있다). 여기서 지우는
    ///   건 **우리 쪽 표**뿐이다 — 죽은 에이전트의 busy 플래그가 남으면 그 이름 앞 파킹이 영영 대기한다.
    /// ★부착 표시를 함께 지우는 게 양성 게이트의 다른 반쪽★: 표시가 사라지면 그 id 의 남은 tap 들은
    ///   더 이상 busy 를 찍지 못한다(mark_busy 게이트) — 죽어가는 core 의 잔여 이벤트가 유령 busy 를
    ///   되살리는 경로가 구조적으로 닫힌다(fix 9).
    pub fn forget(&self, id: PeerId) {
        self.shared.forget(id);
    }

    /// 이 (id, epoch) 가 관측상 턴 중인가(부재 = idle — positive-knowledge-only).
    pub fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
        self.shared.is_busy((id, epoch))
    }

    /// 턴 신호 수신구 조립(내부) — 상태만 공유한다(tracker 자체를 잡지 않는다).
    fn make_probe(&self) -> Arc<TurnProbe> {
        Arc::new(TurnProbe {
            shared: self.shared.clone(),
        })
    }

    /// ★하네스 전용★ — subscribe 를 거치지 않고 턴 신호 수신구만 만든다. 통합 테스트가 실 claude 턴 없이
    ///   턴 신호를 **직접 주입**해 idle 게이트·배치 flush 를 결정적으로 구동하려고 쓴다(spec §7 배치
    ///   검증 강화). 운영 경로는 `attach` 만 쓴다(부착 표시·중복 방지를 거치는 유일한 문).
    ///
    /// ★함께 `mark_attached_for_test` 를 불러야 한다★: 양성 attach 게이트(fix 9) 때문에 부착 표시가 없는
    ///   키의 busy 관측은 무시된다 — 하네스 수신구도 그 게이트를 통과해야 상태가 움직인다.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn probe_for_test(&self) -> Arc<TurnProbe> {
        self.make_probe()
    }

    /// ★하네스/테스트 전용★ — subscribe 없이 **부착 표시만** 등록한다(양성 게이트 통과용).
    ///   운영 경로는 절대 부르지 않는다(tap 없는 부착 표시 = 관측되지 않는 busy 게이트).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn mark_attached_for_test(&self, id: PeerId, epoch: u32) {
        self.shared.claim_attached((id, epoch));
    }

    /// ★하네스/테스트 전용★ — 현재 부착 수(중복 부착 방지 단언용).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn attached_len(&self) -> usize {
        self.shared.attached_len()
    }

    /// ★하네스/테스트 전용★ — busy 관측 시각을 **지정해** 표에 넣는다(양성 attach 게이트는 그대로 통과해야
    ///   한다). 실시간 30분을 기다리지 않고 `BUSY_MAX_TURN` 경계·시각 갱신을 결정적으로 단언하려는 seam.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn mark_busy_at_for_test(&self, id: PeerId, epoch: u32, at: Instant) {
        self.shared.mark_busy_at((id, epoch), at);
    }
}

impl BusyGate for BusyTracker {
    fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
        BusyTracker::is_busy(self, id, epoch)
    }
}

/// ★TurnProbe — 턴 신호 수신구(C2 · ADR-0110 결정 4)★: 호스트 어댑터가 "이 참여자의 턴이 **진행**
///   중이다 / **끝났다**" 를 알려 넣는 두 구멍. `BusyTracker::attach` 가 만들어 `TapHost` 에 건네고,
///   호스트는 자기 출력 계층에 붙어 이벤트를 분류한 뒤 이 메서드들을 부른다.
///
/// ★여기 없는 것 = 분류(load-bearing 경계, ADR-0110 결정 4 · ADR-0004 와 같은 결)★: "어떤 출력 이벤트가
///   턴 진행이고 어떤 게 턴 종료인가" 는 백엔드(claude stream-json)의 지식이다. 그걸 커널에 두면 이
///   crate 가 core 의 출력 타입을 알아야 해 완전 상호무지가 깨진다. 그래서 분류는 데몬 어댑터
///   (`messaging_host::TurnTapSink`)가 하고, 커널은 **신호 어휘 두 개**만 소유한다. busy 정책(양성 attach
///   게이트·positive-knowledge-only·상한 sweep·통지 규율)은 전부 이쪽(`TurnStates`)에 남는다 — 포트는
///   얇게, 정책은 커널에.
///
/// ★콜백 규율(load-bearing — 절대 위반 금지)★: 두 메서드는 **호스트의 출력 pump 스레드**가 부르는 동기
///   콜백이다. 하는 일은 ① 작은 락 구간의 표 갱신 ② 논블록 통지 send **둘뿐** — 주입·호스트 호출·
///   messaging 락 취득·blocking IO 를 하지 않는다(모듈 헤더 콜백 규율).
pub struct TurnProbe {
    shared: Arc<TurnStates>,
}

impl TurnProbe {
    /// 턴 **진행** 신호 — 이 (참여자, epoch) 가 지금 턴 중이라는 관측을 넣는다(양성 attach 게이트를
    ///   통과한 키만 실제로 기록된다 — `TurnStates::mark_busy`).
    pub fn on_progress(&self, peer: PeerId, epoch: u32) {
        self.shared.mark_busy((peer, epoch));
    }

    /// 턴 **종료** 신호 — busy 해제 + flush 도어벨 통지(`IdleNotifier`). 전이 여부와 무관하게 매번
    ///   통지한다(누락 < 잉여 — `IdleNotifier` idempotency 주석).
    pub fn on_turn_done(&self, peer: PeerId, epoch: u32) {
        self.shared.mark_idle((peer, epoch));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// 통지 기록용 IdleNotifier — 통지된 PeerId 순서를 모은다.
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

    /// subscribe 를 기록/스크립트하는 TapHost — 실 manager·PTY 없이 부착 정책만 단언한다.
    struct FakeTapHost {
        calls: StdMutex<Vec<PeerId>>,
        /// true 면 subscribe 를 Err 로(이미 죽은 에이전트 모사).
        fail: StdMutex<bool>,
        /// true 면 subscribe 안에서 **패닉**(fix 8b 롤백 가드 검증 — core 락 구간 패닉 모사).
        panic_in_subscribe: StdMutex<bool>,
        /// 배선된 수신구 보관 — 테스트가 턴 신호를 직접 넣는다.
        probes: StdMutex<Vec<Arc<TurnProbe>>>,
        /// 로스터 현재 epoch(fix 6 검증용). 없는 id 는 부재(None) 취급.
        current: StdMutex<std::collections::HashMap<PeerId, u32>>,
        /// Some(e) 면 **subscribe 진입 직후** 현재 epoch 을 e 로 바꾼다(1회) — attach 사전 검증과 구독 사이에
        ///   재시작이 끼는 창(round-3 finding 5)을 결정적으로 재현한다. 운영 host 는 같은 지점에서 현재
        ///   epoch 을 재확인하므로, 이 hook 이 그 검증을 실제로 구동한다.
        flip_at_subscribe: StdMutex<Option<u32>>,
    }
    impl FakeTapHost {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: StdMutex::new(Vec::new()),
                fail: StdMutex::new(false),
                panic_in_subscribe: StdMutex::new(false),
                probes: StdMutex::new(Vec::new()),
                current: StdMutex::new(std::collections::HashMap::new()),
                flip_at_subscribe: StdMutex::new(None),
            })
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
        fn set_fail(&self, v: bool) {
            *self.fail.lock().unwrap() = v;
        }
        fn set_panic(&self, v: bool) {
            *self.panic_in_subscribe.lock().unwrap() = v;
        }
        fn set_current(&self, id: PeerId, epoch: u32) {
            self.current.lock().unwrap().insert(id, epoch);
        }
        /// subscribe 진입 직후 현재 epoch 을 바꾸도록 무장(attach 사전검증↔구독 사이 재시작 모사).
        fn arm_epoch_flip_at_subscribe(&self, epoch: u32) {
            *self.flip_at_subscribe.lock().unwrap() = Some(epoch);
        }
        fn last_probe(&self) -> Arc<TurnProbe> {
            self.probes
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("no probe")
        }
    }
    impl TapHost for FakeTapHost {
        fn subscribe_output(
            &self,
            id: PeerId,
            expect_epoch: u32,
            probe: Arc<TurnProbe>,
        ) -> Result<(), SubscribeError> {
            self.calls.lock().unwrap().push(id);
            // 재시작 창 모사(1회) — 운영 host 와 같은 지점에서 현재 epoch 을 재확인한다.
            if let Some(new_epoch) = self.flip_at_subscribe.lock().unwrap().take() {
                self.current.lock().unwrap().insert(id, new_epoch);
            }
            let current = self.current.lock().unwrap().get(&id).copied();
            if current != Some(expect_epoch) {
                // 운영 host 는 여기서 방금 만든 구독을 회수한다 — fake 는 애초에 수신구를 보관하지 않는다.
                return Err(SubscribeError::StaleEpoch { current });
            }
            if *self.panic_in_subscribe.lock().unwrap() {
                panic!("fake: subscribe panicked (의도된 패닉 — 롤백 가드 검증)");
            }
            if *self.fail.lock().unwrap() {
                return Err(SubscribeError::Failed("fake: agent gone".to_string()));
            }
            self.probes.lock().unwrap().push(probe);
            Ok(())
        }
        fn current_epoch(&self, id: PeerId) -> Option<u32> {
            self.current.lock().unwrap().get(&id).copied()
        }
    }

    fn tracker() -> (Arc<BusyTracker>, Arc<FakeTapHost>, Arc<RecordingNotifier>) {
        let host = FakeTapHost::new();
        let notifier = RecordingNotifier::new();
        let t = Arc::new(BusyTracker::new(host.clone(), notifier.clone()));
        (t, host, notifier)
    }

    /// 로스터에 그 epoch 을 등록한 뒤 attach — fix 6(epoch 검증)이 통과하는 정상 부착 경로.
    fn attach_live(
        t: &Arc<BusyTracker>,
        h: &Arc<FakeTapHost>,
        id: PeerId,
        epoch: u32,
    ) -> AttachOutcome {
        h.set_current(id, epoch);
        t.attach(id, epoch)
    }

    #[test]
    fn unknown_agent_is_idle_positive_knowledge_only() {
        // ★폴백 불변식(ADR-0104)★: 관측 근거가 없으면 busy 가 아니다(= 즉시 주입).
        let (t, _h, _n) = tracker();
        assert!(
            !t.is_busy(PeerId::new_v4(), 0),
            "미관측 대상은 idle 취급(관측 불가 백엔드 즉시 주입 폴백)"
        );
    }

    #[test]
    fn delta_marks_busy_and_done_marks_idle() {
        let (t, _h, _n) = tracker();
        let id = PeerId::new_v4();
        t.mark_attached_for_test(id, 0); // 양성 attach 게이트(fix 9) 통과.
        let tap = t.probe_for_test();
        tap.on_progress(id, 0);
        assert!(t.is_busy(id, 0), "TextDelta = 턴 진행 관측 → busy");
        tap.on_turn_done(id, 0);
        assert!(!t.is_busy(id, 0), "MessageDone = 턴 종료 → idle");
    }

    #[test]
    fn every_message_done_notifies_even_without_transition() {
        // ★누락 < 잉여(IdleNotifier 주석)★: 통지는 전이 여부를 따지지 않고 MessageDone 마다 나간다 —
        //   어시스턴트 이벤트 없이 곧장 끝나는 턴에서 통지가 빠지면 파킹이 다음 턴까지 stranded 되므로,
        //   잉여 통지(빈 큐 drain = no-op)를 택했다. 채널 압력은 ws.rs 의 id 별 coalescing 이 묶는다.
        let (t, _h, n) = tracker();
        let id = PeerId::new_v4();
        t.mark_attached_for_test(id, 0);
        let tap = t.probe_for_test();
        tap.on_progress(id, 0);
        tap.on_turn_done(id, 0);
        tap.on_progress(id, 0);
        tap.on_turn_done(id, 0);
        tap.on_turn_done(id, 0);
        assert!(!t.is_busy(id, 0), "마지막 관측이 MessageDone → idle");
        assert_eq!(
            n.seen(),
            vec![id, id, id],
            "MessageDone 3회 = 통지 3회(전이 없는 마지막 것도 통지)"
        );
    }

    #[test]
    fn unattached_tap_cannot_mark_busy_positive_attach_gate() {
        // ★fix 9 회귀★: 부착 표시가 없는 키의 busy 관측은 무시된다. 이게 없으면 rotation 후 살아남은
        //   옛 core 의 잔여 이벤트가 아무도 지우지 않는 유령 busy 를 되살릴 수 있다(TTL 까지 배달 정지).
        let (t, _h, n) = tracker();
        let id = PeerId::new_v4();
        let tap = t.probe_for_test(); // 부착 표시 없음.
        tap.on_progress(id, 0);
        assert!(
            !t.is_busy(id, 0),
            "미부착 키의 turn 관측은 표에 들어가지 않는다(양성 게이트)"
        );
        // 종료 관측은 게이트를 걸지 않는다(제거·통지는 잉여여도 안전 — 누락이 치명).
        tap.on_turn_done(id, 0);
        assert_eq!(n.seen(), vec![id], "미부착이어도 종료 통지는 나간다");
        // 부착되면 그때부터 관측이 반영된다.
        t.mark_attached_for_test(id, 0);
        tap.on_progress(id, 0);
        assert!(t.is_busy(id, 0), "부착 후에는 busy 관측 반영");
    }

    #[test]
    fn straggler_old_epoch_core_cannot_recreate_busy_after_rotation() {
        // ★fix 9 핵심 시나리오★: epoch 0 tap 이 살아 있는 채로 epoch 1 이 부착되면(rotation), 옛 tap 의
        //   지연 이벤트는 (id, 0) 을 되살리지 못해야 한다 — 되살아나면 그 항목은 MessageDone 도 Detach 도
        //   지우지 않는다(Detach 는 id 전체를 지우고 끝난 뒤라서).
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        assert_eq!(attach_live(&t, &h, id, 0), AttachOutcome::Attached);
        let old_probe = h.last_probe();
        old_probe.on_progress(id, 0);
        assert!(t.is_busy(id, 0), "부착 중엔 정상 관측");
        // 재시작(epoch 1) → 부착 표시가 epoch 1 로 교체되고 옛 busy 표시는 청소된다.
        assert_eq!(attach_live(&t, &h, id, 1), AttachOutcome::Attached);
        assert!(!t.is_busy(id, 0));
        // 옛 core 의 잔여 이벤트 — 게이트가 막는다.
        old_probe.on_progress(id, 0);
        assert!(
            !t.is_busy(id, 0),
            "rotation 후 옛 epoch 이벤트는 busy 를 되살리지 못한다"
        );
    }

    #[test]
    fn attach_skips_when_queued_epoch_is_no_longer_current() {
        // ★fix 6 회귀★: Attach 는 채널을 타고 오므로 집행 시점에 stale 할 수 있다 — 현재 epoch 과
        //   다르면 붙지 않는다(유령 부착/중복 tap 금지). 현재 epoch 용 Attach 가 뒤따라 온다.
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        h.set_current(id, 2); // 실제로는 이미 epoch 2.
        assert_eq!(t.attach(id, 1), AttachOutcome::Stale, "stale epoch = skip");
        assert_eq!(h.call_count(), 0, "subscribe 자체를 시도하지 않는다");
        assert_eq!(t.attached_len(), 0, "부착 표시도 남기지 않는다");
        // 부재(reap 완료)도 같은 판정.
        assert_eq!(t.attach(PeerId::new_v4(), 0), AttachOutcome::Stale);
        // 현재 epoch 이면 정상 부착.
        assert_eq!(t.attach(id, 2), AttachOutcome::Attached);
        assert_eq!(h.call_count(), 1);
    }

    #[test]
    fn attach_is_stale_when_epoch_flips_between_precheck_and_subscribe() {
        // ★round-3 finding 5 회귀★: 사전 검증은 통과했으나 **구독 지점**에서 epoch 이 이미 바뀐 경우
        //   (그 사이 재시작). 예전엔 그대로 붙어서 ① 새 epoch core 에 유령 tap 이 남고(tap 은 스스로
        //   빠지지 않는다) ② 부착 표시는 옛 epoch 이라 그 tap 의 관측이 전부 버려지고 ③ 뒤이은 정상
        //   attach 가 sink 를 하나 더 붙여 통지가 중복됐다. 이제 Stale 로 접고 표시도 남기지 않는다.
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        h.set_current(id, 0); // 사전 검증은 통과(현재 epoch 0).
        h.arm_epoch_flip_at_subscribe(1); // 구독 진입 직후 epoch 1 로 교체.
        assert_eq!(
            t.attach(id, 0),
            AttachOutcome::Stale,
            "구독 지점 epoch 불일치 = Stale"
        );
        assert_eq!(h.call_count(), 1, "구독을 시도했다(그 지점에서 잡아냄)");
        assert_eq!(t.attached_len(), 0, "부착 표시를 남기지 않는다(유령 금지)");
        // 현재 epoch(1) 용 Attach 는 정상 부착돼야 한다.
        assert_eq!(t.attach(id, 1), AttachOutcome::Attached);
        assert_eq!(t.attached_len(), 1);
    }

    #[test]
    fn stale_busy_is_swept_and_wakes_the_agent() {
        // ★round-3 finding 4 회귀★: MessageDone 이 영영 오지 않는 턴(파싱 실패·decoder 이상)은 busy 를
        //   영구화해 그 수신자 앞 배달을 TTL 까지 막는다. 상한 sweep 이 표를 지우고 **도어벨을 눌러야**
        //   대기 메일이 나간다(지우기만 하면 다음 트리거가 없어 그대로 앉아 있다).
        let (t, _h, n) = tracker();
        let id = PeerId::new_v4();
        t.mark_attached_for_test(id, 0);
        let tap = t.probe_for_test();
        tap.on_progress(id, 0); // busy 관측(해제 통지는 오지 않는다).
        assert!(t.is_busy(id, 0));
        // 상한 이전 = 청소하지 않는다(정상 진행 중인 턴을 자르면 턴 중 주입이 된다).
        assert_eq!(
            t.sweep_stale_busy(Instant::now() + BUSY_MAX_TURN - Duration::from_secs(1)),
            0,
            "상한 미달은 유지"
        );
        assert!(t.is_busy(id, 0));
        assert!(n.seen().is_empty(), "유지 시 통지도 없다");
        // 상한 경과 = 청소 + 통지 1회.
        assert_eq!(
            t.sweep_stale_busy(Instant::now() + BUSY_MAX_TURN + Duration::from_secs(1)),
            1
        );
        assert!(!t.is_busy(id, 0), "상한 초과 busy 잔해 청소");
        assert_eq!(n.seen(), vec![id], "청소한 id 를 도어벨로 깨운다");
    }

    #[test]
    fn busy_timestamp_refreshes_on_every_turn_observation() {
        // ★상한이 정상 장기 턴을 자르지 않는 이유(load-bearing — BUSY_MAX_TURN 주석)★: 관측마다 시각이
        //   갱신되므로 "출력이 계속 오는 턴" 은 늙지 않는다. 갱신이 없으면(턴 시작 시각 고정) 30분 넘게
        //   도는 정상 턴이 잘려 턴 중 주입이 된다.
        let (t, _h, _n) = tracker();
        let id = PeerId::new_v4();
        t.mark_attached_for_test(id, 0);
        let t0 = Instant::now();
        t.mark_busy_at_for_test(id, 0, t0);
        // 20분 뒤 새 관측(= 갱신). 이 갱신이 없다면 아래 sweep(t0 + MAX)이 청소해 버린다.
        t.mark_busy_at_for_test(id, 0, t0 + Duration::from_secs(20 * 60));
        assert_eq!(
            t.sweep_stale_busy(t0 + BUSY_MAX_TURN),
            0,
            "마지막 관측 기준으로 나이를 재야(갱신 반영) 정상 장기 턴이 안 잘린다"
        );
        assert!(t.is_busy(id, 0));
        // 갱신 시각 + 상한을 넘기면 그때는 청소된다(상한 자체는 살아 있다).
        assert_eq!(
            t.sweep_stale_busy(t0 + Duration::from_secs(20 * 60) + BUSY_MAX_TURN),
            1
        );
    }

    #[test]
    fn sweep_only_clears_and_wakes_stale_entries() {
        // 잉여 도어벨은 무해하지만(빈 큐 no-op) 신선한 busy 를 함께 지우면 그건 **턴 중 주입**이다 — 늙은
        //   항목만 청소하고 그 id 만 깨운다.
        let (t, _h, n) = tracker();
        let stale = PeerId::new_v4();
        let fresh = PeerId::new_v4();
        t.mark_attached_for_test(stale, 0);
        t.mark_attached_for_test(fresh, 0);
        let t0 = Instant::now();
        t.mark_busy_at_for_test(stale, 0, t0);
        t.mark_busy_at_for_test(fresh, 0, t0 + Duration::from_secs(25 * 60));
        assert_eq!(
            t.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            1,
            "늙은 항목만 청소"
        );
        assert!(!t.is_busy(stale, 0));
        assert!(t.is_busy(fresh, 0), "신선한 busy 는 보존");
        assert_eq!(n.seen(), vec![stale]);
    }

    #[test]
    fn attach_marker_is_rolled_back_when_subscribe_panics() {
        // ★fix 8b 회귀★: subscribe 가 패닉으로 unwind 해도 부착 표시가 남으면 그 (id, epoch) 는 영영
        //   tap 없이 돌고(중복 접힘) 양성 게이트 때문에 busy 도 못 찍힌다 = 게이트가 조용히 사라짐.
        //   Drop 가드가 되돌리는지 본다. (이 테스트는 의도된 패닉 메시지를 출력한다.)
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        h.set_current(id, 0);
        h.set_panic(true);
        let t2 = t.clone();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            t2.attach(id, 0);
        }));
        assert!(res.is_err(), "패닉이 호출자에게 전파돼야(worker 가 격리)");
        assert_eq!(t.attached_len(), 0, "패닉 unwind 에서도 부착 표시 롤백");
        // 재시도 가능해야 한다.
        h.set_panic(false);
        assert_eq!(
            t.attach(id, 0),
            AttachOutcome::Attached,
            "다음 요청에 재시도"
        );
    }

    #[test]
    fn state_is_keyed_by_epoch_no_leak_across_incarnations() {
        // epoch 교체(재스폰) = 새 OutputCore. 옛 epoch 의 busy 가 새 epoch 판정에 새면 새 incarnation 앞
        //   메일이 근거 없이 대기한다 — 키에 epoch 을 넣어 구조적으로 막는다.
        let (t, _h, _n) = tracker();
        let id = PeerId::new_v4();
        t.mark_attached_for_test(id, 0);
        let tap = t.probe_for_test();
        tap.on_progress(id, 0);
        assert!(t.is_busy(id, 0));
        assert!(
            !t.is_busy(id, 1),
            "다른 epoch 은 미관측 → idle(옛 incarnation busy 누수 금지)"
        );
    }

    #[test]
    fn attach_dedups_same_id_epoch_and_resubscribes_on_epoch_bump() {
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        assert_eq!(attach_live(&t, &h, id, 0), AttachOutcome::Attached);
        assert_eq!(t.attach(id, 0), AttachOutcome::AlreadyAttached);
        assert_eq!(t.attach(id, 0), AttachOutcome::AlreadyAttached);
        assert_eq!(h.call_count(), 1, "같은 (id, epoch) 중복 subscribe 금지");
        assert_eq!(t.attached_len(), 1);
        // epoch bump = 새 OutputCore → 다시 붙어야 한다(구독은 epoch 을 넘지 못한다).
        assert_eq!(attach_live(&t, &h, id, 1), AttachOutcome::Attached);
        assert_eq!(h.call_count(), 2, "epoch bump 은 재-subscribe");
        assert_eq!(
            t.attached_len(),
            1,
            "옛 epoch 부착 표시는 청소(한 id 에 살아있는 epoch 은 하나 — 누적 누수 차단)"
        );
    }

    #[test]
    fn attach_on_epoch_bump_clears_stale_epoch_busy_flag() {
        // 재시작(epoch bump)하면 옛 incarnation 의 busy 표시는 무의미하다 — 남겨 두면 항목이 누적되고,
        //   같은 epoch 이 재사용되는 상황(맵 교체 순서)에서 근거 없는 busy 로 오판할 수 있다.
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        attach_live(&t, &h, id, 0);
        let tap = t.probe_for_test();
        tap.on_progress(id, 0);
        assert!(t.is_busy(id, 0));
        // 재스폰(epoch 1)로 재부착 → 옛 epoch 의 busy 표시 소멸.
        attach_live(&t, &h, id, 1);
        assert!(!t.is_busy(id, 0), "옛 epoch busy 표시 청소");
        assert!(!t.is_busy(id, 1), "새 epoch 은 아직 미관측 = idle");
    }

    #[test]
    fn attach_failure_rolls_back_marker_and_reports_failed() {
        // 이미 죽은 에이전트면 subscribe 가 Err → ① 부착 표시 롤백 ② `Failed` 반환(호출자가 로스터 diff
        //   스냅샷을 무효화해 재시도를 열어야 한다 — fix 8a). 표시가 남으면 그 (id, epoch) 는 영영 tap 없이
        //   idle 폴백으로만 돈다.
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        h.set_fail(true);
        assert_eq!(attach_live(&t, &h, id, 0), AttachOutcome::Failed);
        assert_eq!(h.call_count(), 1);
        assert_eq!(t.attached_len(), 0, "실패 시 부착 표시 롤백");
        h.set_fail(false);
        assert_eq!(t.attach(id, 0), AttachOutcome::Attached, "재시도 가능");
        assert_eq!(h.call_count(), 2);
        assert_eq!(t.attached_len(), 1);
    }

    #[test]
    fn attached_tap_drives_state_through_host() {
        // attach 로 배선된 수신구가 실제로 이 tracker 의 상태를 갱신하는지(make_probe↔shared 배선 확인).
        let (t, h, n) = tracker();
        let id = PeerId::new_v4();
        attach_live(&t, &h, id, 2);
        let probe = h.last_probe();
        probe.on_progress(id, 2);
        assert!(t.is_busy(id, 2));
        probe.on_turn_done(id, 2);
        assert!(!t.is_busy(id, 2));
        assert_eq!(n.seen(), vec![id]);
    }

    #[test]
    fn forget_clears_state_and_attachment_for_departed_agent() {
        // 로스터 이탈(죽음) → busy 플래그와 부착 표시 청소. 안 지우면 죽은 수신자 앞 파킹이 영영 대기한다.
        let (t, h, _n) = tracker();
        let id = PeerId::new_v4();
        let other = PeerId::new_v4();
        attach_live(&t, &h, id, 0);
        attach_live(&t, &h, other, 0);
        // 부착 표시는 (id,0)·(other,0) 2개. epoch 0 관측으로 둘 다 busy.
        let tap = t.probe_for_test();
        tap.on_progress(id, 0);
        tap.on_progress(other, 0);
        assert!(t.is_busy(id, 0) && t.is_busy(other, 0));

        t.forget(id);
        assert!(!t.is_busy(id, 0), "이탈 에이전트 busy 플래그 청소");
        assert!(t.is_busy(other, 0), "다른 에이전트 상태는 보존");
        assert_eq!(t.attached_len(), 1, "이탈 id 의 부착 표시 제거");
        // 다시 등장하면 재부착된다(표시가 지워졌으므로).
        assert_eq!(attach_live(&t, &h, id, 0), AttachOutcome::Attached);
        assert_eq!(h.call_count(), 3);
    }

    #[test]
    fn always_idle_gate_never_reports_busy() {
        // 폴백 게이트 — 게이트 미배선 조립이 C1 과 동일하게(즉시 주입) 돌게 하는 안전 기본값.
        let g = AlwaysIdleGate;
        assert!(!g.is_busy(PeerId::new_v4(), 7));
    }
}
