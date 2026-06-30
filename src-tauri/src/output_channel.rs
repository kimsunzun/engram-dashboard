//! window Channel registry + agent 공유 출력 버퍼(S14 모듈① 출력 평면 재설계 2단계, ADR-0040).
//!
//! ## 왜 두 조각인가 (registry ≠ buffer — 락 분리)
//! - **`WindowChannelRegistry`** = `window_label → 출력 Channel`(Tauri 타입 보관). 창 mount 시
//!   `subscribe_output` invoke 가 insert, dead window(send Err) 감지 시 connection task 가 remove.
//! - **`AgentBufferStore`** = `Arc<Mutex<OutputViewStore<WindowLabel>>>`(core 순수 store 를 Tauri 결합부로
//!   감쌈). 에이전트당 공유 콘텐츠 1벌 + 창별 cursor + per-agent epoch 태그를 든다.
//!
//! 둘을 **분리한 락**으로 둔 이유(TRD §1 락 규율 — ★/review code deep 급소★): on_frame 은 buffer 락
//! 안에서 "어느 창에 어떤 bytes 를 보낼지" snapshot(`Vec<(WindowLabel, Vec<u8>)>`)만 수집하고, **buffer 락을
//! 푼 뒤** registry 락을 잡아 Channel `send` 한다. buffer 락 ⊃ registry 락 중첩이 0 이라 두 경로(fan-out·
//! subscribe)의 락 순서 역전 데드락이 구조적으로 불가능하다(데몬 `output_core` C4 패턴과 동형, ADR-0006).
//!
//! ## ★raw byte 함정(spike §7)★
//! 출력 프레임은 `Channel<tauri::ipc::Response>` 로 운반한다 — `Channel<Vec<u8>>`/`Channel<&[u8]>` 는
//! blanket `impl<T:Serialize> IpcResponse` 가 JSON 배열로 직렬화해 바이트가 샌다. 반드시
//! `Response::new(bytes)` 로 실어 raw 로 보낸다.
//!
//! ## ★동시성(load-bearing)★
//! `std::sync::Mutex` 다(tokio Mutex 아님). connection task(tokio)가 핫패스에서 lock→snapshot/send 하는데
//! 락 보유 중 `.await` 가 **없다**(`Channel::send` 는 동기, store 메서드도 동기). 락은 짧게 잡았다 즉시 푼다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::output_router::WindowLabel;
use engram_dashboard_core::output_view_store::OutputViewStore;
use engram_dashboard_protocol::AgentId;

/// window label → 그 창의 출력 Channel. 창 mount 시 `subscribe_output` invoke 가 insert,
/// dead window(send Err) 감지 시 connection task 가 remove 한다. connection task 와 Tauri command
/// 양쪽이 `Arc` 로 공유 → app-level manage.
///
/// ★T7b→2단계 변경★: 옛 `WindowEntry`(Channel + per-window `WindowSeqTracker`)에서 seq 추적을
/// 떼어냈다 — dedup/cursor 는 이제 `AgentBufferStore`(공유 버퍼 + per-view cursor)가 단독 소유한다.
/// registry 는 순수하게 "label → Channel" 만 든다(min 모델 폐기, 두 모델 혼재 차단).
pub type WindowChannelRegistry =
    Arc<Mutex<HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>>>;

/// 슬롯 키 = `(WindowLabel, AgentId)` 쌍 = "한 창이 보는 한 agent".
///
/// ★왜 창(WindowLabel) 단독이 아니라 (창, agent) 쌍인가(결정 근거)★: 현재 fan-out 단위는 창
/// (registry = `WindowLabel → Channel` 하나)이지만, **한 창은 split 으로 여러 agent 를 동시에 본다**
/// (router `agent → [WindowLabel]` 역인덱스, 한 창이 여러 agent 의 targets 에 등장). 같은 Channel 로
/// 여러 agent frame 이 흐르고 프론트가 헤더 agent_id 로 분기한다. 따라서 cursor(진도)는 창당 하나가
/// 아니라 **(창, agent) 쌍당 하나** 여야 무손실/dedup 이 agent 별로 독립한다. 1단계 `SlotCursorMap<S>`
/// 가 "S 당 agent 하나 + slots_for_agent 역조회" 라 `S=(WindowLabel, AgentId)` 와 정확히 맞는다.
pub type ViewSlotKey = (WindowLabel, AgentId);

/// 에이전트당 공유 출력 버퍼 store. 슬롯 키 = [`ViewSlotKey`].
///
/// connection task fan-out(on_frame)·재연결 resubscribe(resubscribe_after_seq)가 *동일* 인스턴스를
/// 본다(lib.rs setup 이 만든 공유 Arc). cursor 생명주기(어느 창이 어느 agent 를 보나)는 on_frame 직전
/// `sync_viewers`(router 스냅샷 파생)로 동기화한다(connection.rs Binary arm).
pub type AgentBufferStore = Arc<Mutex<OutputViewStore<ViewSlotKey>>>;

/// 새 빈 버퍼 store(lib.rs setup 이 1회 생성해 app.manage + DaemonClient 주입).
pub fn new_buffer_store() -> AgentBufferStore {
    Arc::new(Mutex::new(OutputViewStore::new()))
}

/// ★현재 Channel 이 등록된 window label 집합 snapshot(근원2 — deliverable 게이트)★. on_frame 직전에
/// 떠서 store 에 주입한다 — store(core)는 registry 를 모르므로(ADR-0012), "어느 창이 실제로 Channel 을
/// 들고 있나"를 src-tauri 가 알려줘야 cursor advance ⟺ delivery 불변식을 세운다(미mount 창은 advance 금지).
///
/// ## ★ADR-0006 락 규율(load-bearing — buffer 락 ⊃ registry 락 중첩 0)★
/// 이 함수는 **buffer 락을 잡기 *전*에** 짧게 registry 락을 잡아 label 집합만 clone 하고 즉시 푼다 —
/// on_frame(buffer 락) 안에서 registry 락을 잡으면 flush_snapshot(역시 registry 락) 경로와 락 순서가
/// 역전돼 데드락 표면이 생긴다. 그래서 registry → (락 해제) → buffer → (락 해제) → flush(registry) 순서로
/// 락이 절대 중첩되지 않게 한다. label 수는 창 수(소수)라 clone 비용 미미.
pub fn registered_labels(
    registry: &WindowChannelRegistry,
) -> std::collections::HashSet<WindowLabel> {
    match registry.lock() {
        Ok(reg) => reg.keys().cloned().collect(),
        // poisoned — 빈 집합 반환(아무 slot 도 deliverable 아님 → advance 0, 안전 측). on_frame 은 콘텐츠
        //   append 는 하므로(deliverable 무관) 축 A 무손실은 유지, 전달만 다음 정상 frame 까지 미뤄진다.
        Err(_) => {
            tracing::warn!("registry lock poisoned — deliverable 집합 빈값(advance 0, 안전 측)");
            std::collections::HashSet::new()
        }
    }
}

/// ★deliverable 집합 빌드의 *순수 필터* 부분(FIX-B — 3 호출부 복붙 제거 + 단위테스트 가능)★. `slots` =
/// 이번에 다룰 `(label, agent)` slot 목록, `registered` = 현재 Channel 이 registry 에 등록된 label 집합
/// ([`registered_labels`] 가 락 조회해 떠 줌). slot 중 그 label 이 registered 에 든 것만 골라
/// `HashSet<ViewSlotKey>` deliverable 로 모은다. on_frame/reconcile/ReplaySlots 세 호출부가 *같은* 필터
/// (label 등록 여부)를 쓰던 걸 한 곳으로 모은다(drift 차단).
///
/// ## ★락 규율은 호출부가 쥔다 — 이 함수는 락을 모른다(ADR-0006)★
/// 이 함수는 **순수**다(registry 락 조회 없음, 입력은 이미 떠 온 자료뿐). registry 락은 호출부가
/// **buffer 락을 잡기 *전*에** [`registered_labels`] 로 짧게 잡았다 풀고(buffer 락 ⊃ registry 락 중첩 0),
/// 그 결과 집합을 이 함수에 넘긴다. 순수 필터만 떼어내 락 조회 타이밍은 현행 그대로 — 이 분리가 락
/// 순서를 바꾸지 않는다. 그래서 src-tauri 의존(Tauri 객체·락) 없이 단위테스트 가능하다.
pub fn build_deliverable(
    slots: &[ViewSlotKey],
    registered: &std::collections::HashSet<WindowLabel>,
) -> std::collections::HashSet<ViewSlotKey> {
    slots
        .iter()
        .filter(|(label, _)| registered.contains(label))
        .cloned()
        .collect()
}

/// 버퍼 store 가 돌려준 `((WindowLabel, AgentId), bytes)` snapshot 을 **buffer 락 밖에서** 각 창 Channel 로
/// 보낸다. slot 키의 첫 요소(WindowLabel)로 registry 에서 Channel 을 찾는다(둘째 요소=AgentId 는 cursor
/// 분리용일 뿐 — Channel 은 창당 하나라 같은 창의 여러 agent frame 은 같은 Channel 로 합류, 프론트가
/// 헤더 agent_id 로 분기).
///
/// ## ★ADR-0006(load-bearing — /review code deep 급소)★
/// 이 함수는 **buffer 락을 보유하지 않은 상태에서만** 불려야 한다(호출자가 store.lock() snapshot 을
/// 받고 그 락을 drop 한 뒤 호출). 여기서 잡는 건 registry 락뿐이고, 그 안에 `.await` 가 0 이다
/// (`Channel::send` 동기). dead window(`send` Err) 라벨은 같은 lock 안에서 모았다가 곧바로 remove 한다
/// (절대 unwrap 금지 — 소멸 webview 는 Channel send 가 Err, spike §7 D6).
///
/// ★bytes 소유★: `Response::new` 는 `Vec<u8>` 소유가 필요하다. snapshot 이 이미 `Vec<u8>` 를 들고
/// 오므로(store 가 `.to_vec()` 떠 줌) 여기선 그대로 `Response::new(bytes)` 로 move 한다(추가 clone 0).
pub fn flush_snapshot(
    registry: &WindowChannelRegistry,
    snapshot: Vec<(ViewSlotKey, Vec<u8>)>,
    my_gen: u64,
) {
    if snapshot.is_empty() {
        return;
    }
    let mut dead: Vec<WindowLabel> = Vec::new();
    {
        // ★락 across await 없음★: 이 블록 안에 .await 0 — Channel::send 는 동기.
        let Ok(mut reg) = registry.lock() else {
            tracing::warn!(
                generation = my_gen,
                "registry lock poisoned — 출력 snapshot flush 스킵"
            );
            return;
        };
        for ((label, _agent_id), bytes) in snapshot {
            if let Some(channel) = reg.get(&label) {
                if channel.send(tauri::ipc::Response::new(bytes)).is_err() {
                    // 소멸 webview — registry 에서 제거 대상(절대 unwrap 금지, spike §7 D6).
                    dead.push(label);
                }
            } else {
                // ★label 이 registry 에 없음 — 근원2 FIX 후로는 정상 흐름에서 거의 안 닿는다★: on_frame 이
                //   deliverable(=현재 등록된 label) 게이트로 등록된 slot 만 snapshot 에 담으므로, 여기 오는
                //   label 은 대부분 등록돼 있다. 그래도 미세 race(snapshot 수집과 이 flush 사이에 dead 감지로
                //   막 remove 된 창, 또는 ReplaySlots/reconcile 가 등록 직전 slot 에 replay 를 먼저 낸 경우)는
                //   남을 수 있어 방어적으로 스킵한다 — 그 경우 그 창은 곧 재mount(subscribe_output 의 fresh
                //   리셋+replay)로 무손실 복구되거나, dead 면 더는 viewer 아니라 무해(다음 layout rebuild 의
                //   drop_slots/reconcile sweep 가 cursor 정리). 즉 deliverable 게이트가 "advance 했는데 영구
                //   miss"의 주 경로를 닫고, 여기 스킵은 잔여 race 의 안전망이다.
                //   ★FIX-3 가시화(debug)★: 옛 무로그 스킵은 유실을 침묵시켰다 — registry miss 를 debug 로
                //   드러내 디버깅 단서를 남긴다(dead webview remove 는 정상이라 warn 아닌 debug 로 충분).
                tracing::debug!(
                    generation = my_gen,
                    %label,
                    "flush_snapshot: registry 미등록 label 스킵(미mount/막 remove — 재mount 가 복구)"
                );
            }
        }
        // dead label 제거(같은 lock 보유 중 — 동기).
        for label in &dead {
            reg.remove(label);
        }
    } // ← registry lock drop
    if !dead.is_empty() {
        tracing::debug!(generation = my_gen, dead = ?dead, "dead window Channel 제거");
    }
}

#[cfg(test)]
mod tests {
    //! ★`build_deliverable` 순수 필터 단위테스트(FIX-B)★. registry 락·Tauri Channel 없이 자료만으로
    //! 검증한다 — src-tauri lib test 는 WebView2 DLL 링크가 막혀 실행 자체가 안 되지만, 이 함수는
    //! Tauri 타입을 안 들고(입력=slot 슬라이스 + label 집합, 출력=label 등록된 slot 집합) `#[cfg(test)]`
    //! 코드도 그걸 안 닿으므로 `cargo build` 컴파일·`cargo test` 회귀가 정상 동작한다.
    use super::*;
    use engram_dashboard_protocol::AgentId;
    use std::collections::HashSet;

    fn aid(n: u128) -> AgentId {
        AgentId::from_u128(n)
    }

    fn slot(label: &str, agent: AgentId) -> ViewSlotKey {
        (label.to_string(), agent)
    }

    fn registered(labels: &[&str]) -> HashSet<WindowLabel> {
        labels.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn keeps_only_registered_label_slots() {
        // keep slot 중 label 이 registered 에 든 것만 deliverable 로 떨어진다.
        let a = aid(1);
        let slots = vec![slot("main", a), slot("popup", a), slot("tree", a)];
        let reg = registered(&["main", "tree"]); // popup 미등록(미mount)
        let d = build_deliverable(&slots, &reg);
        assert_eq!(d.len(), 2);
        assert!(d.contains(&slot("main", a)));
        assert!(d.contains(&slot("tree", a)));
        assert!(!d.contains(&slot("popup", a)), "미등록 label slot 은 제외");
    }

    #[test]
    fn empty_registered_yields_empty_set() {
        // registry 가 비면(아무 창도 Channel 미등록) deliverable 0 → advance 0(안전 측).
        let a = aid(1);
        let slots = vec![slot("main", a), slot("popup", a)];
        let d = build_deliverable(&slots, &HashSet::new());
        assert!(d.is_empty(), "registered 비면 빈 집합");
    }

    #[test]
    fn empty_slots_yields_empty_set() {
        // 다룰 slot 이 없으면(이번 frame/reconcile 대상 0) registered 가 차 있어도 deliverable 0.
        let d = build_deliverable(&[], &registered(&["main", "popup"]));
        assert!(d.is_empty(), "slot 비면 빈 집합");
    }

    #[test]
    fn same_label_different_agents_both_survive() {
        // ★ViewSlotKey 단위 보존★: 한 창(같은 label)이 split 으로 두 agent 를 보면 slot 두 개다 —
        //   필터는 label 등록 여부지만 결과 집합은 (label, agent) 단위라 둘 다 살아남아야 한다.
        let (a, b) = (aid(1), aid(2));
        let slots = vec![slot("main", a), slot("main", b)];
        let d = build_deliverable(&slots, &registered(&["main"]));
        assert_eq!(d.len(), 2, "같은 label·다른 agent 두 slot 모두 deliverable");
        assert!(d.contains(&slot("main", a)));
        assert!(d.contains(&slot("main", b)));
    }
}
