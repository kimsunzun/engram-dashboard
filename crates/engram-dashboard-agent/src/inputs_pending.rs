//! inputs_pending — (에이전트, epoch)별 **대기 목록 표**: "이 화신의 사용자 대기 목록이 비지 않았다".
//!
//! ★턴 관측 표(`turn`)와 같은 모양의 잎 표이고, 따로 있는 이유는 쓰는 자리가 달라서다★: 턴 관측 표의
//!   `in_turn` 을 쓰는 자리는 분류기와 정리 두 지점뿐이어야 한다(ADR-0127 「세 번째 호출자」). 이 사실은
//!   코어가 명부를 환원한 emit 안에서 적으므로 그 표에 칸을 더하지 않는다.
//! ★읽는 쪽 = 우편의 바쁨(데몬 어댑터)★ — 이 값이 참인 동안 우편은 들지 않고, ★fail-open 상한이 없다★
//!   (사용자 결정 N8 — ADR-0104 「늦게 가는 것 < 안 가는 것」의 의도된 예외).
//! ★락 = 이 표 하나, leaf★: 잡은 채 밖을 부르지 않는다(ADR-0006). 항목을 `AgentId` 로 키잡고 epoch 은
//!   값으로 든다 — 근거는 `turn` 모듈 헤더와 같다.
// ADR-0231
// ADR-0006

use std::collections::HashMap;
use std::sync::Mutex;

use crate::types::AgentId;

/// 대기 목록 표 — `AgentManager` 가 하나 소유한다.
pub struct InputsPendingTable {
    entries: Mutex<HashMap<AgentId, Entry>>,
}

struct Entry {
    epoch: u32,
    pending: bool,
    /// 이 항목을 마지막으로 갱신한 출력 seq(같은 epoch 안에서만 비교 가능).
    last_seq: u64,
}

impl Default for InputsPendingTable {
    fn default() -> Self {
        Self::new()
    }
}

impl InputsPendingTable {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// poison 내성 — 매니저 전역 표라 한 홀더의 패닉이 전 에이전트로 번지지 않게 한다(`turn` 과 같은 판단).
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 이 화신이 이 id 의 자리를 차지한다 — 있던 항목을 무조건 갈아치우고 "목록 빔" 으로 시작한다.
    /// ★세션이 관측 가능해지기 전에 부른다★(주인 = `AgentManager::spawn_session`, 턴 관측 등록과 같은 자리).
    pub fn register(&self, id: AgentId, epoch: u32) {
        self.lock().insert(
            id,
            Entry {
                epoch,
                pending: false,
                last_seq: 0,
            },
        );
    }

    /// 목록이 비었다↔찼다를 적는다. `seq` = 그 값을 낳은 목록 사건의 출력 seq.
    ///
    /// ★더 작은 seq 의 쓰기를 버린다★: 「비었다」는 replay 락 밖으로 미뤄 적히므로, 그 사이 다른 스레드가
    ///   락 안에서 적은 더 늦은 「찼다」를 덮으면 안 된다.
    /// ★표식이 다르거나 항목이 없으면 버린다(턴 관측 표와 다르다)★: 이 사실에는 상한이 없어서, 정리 뒤에
    ///   늦게 온 쓰기가 항목을 되살리면 그것을 지울 계기가 없다. 그래서 자리를 만드는 동사는 `register` 하나다.
    pub fn set(&self, id: AgentId, epoch: u32, seq: u64, pending: bool) {
        let mut g = self.lock();
        let Some(e) = g.get_mut(&id) else {
            return;
        };
        if e.epoch != epoch || seq < e.last_seq {
            return;
        }
        e.pending = pending;
        e.last_seq = seq;
    }

    /// 이 (에이전트, epoch)의 값. `None` = 항목 없음(다른 epoch 만 있는 경우 포함).
    pub fn get(&self, id: AgentId, epoch: u32) -> Option<bool> {
        self.lock()
            .get(&id)
            .filter(|e| e.epoch == epoch)
            .map(|e| e.pending)
    }

    /// 이 화신의 항목 제거 — epoch 이 일치할 때만(지각한 호출이 새 화신의 항목을 지우지 못하게).
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

    #[test]
    fn a_registered_incarnation_starts_empty_and_follows_writes() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        assert_eq!(t.get(id, 0), None);
        t.register(id, 0);
        assert_eq!(t.get(id, 0), Some(false));
        t.set(id, 0, 3, true);
        assert_eq!(t.get(id, 0), Some(true));
        t.set(id, 0, 4, false);
        assert_eq!(t.get(id, 0), Some(false));
        assert_eq!(t.get(id, 1), None, "다른 epoch 은 항목 없음");
    }

    #[test]
    fn a_deferred_empty_write_with_a_smaller_seq_cannot_cover_a_later_full_one() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.register(id, 0);
        t.set(id, 0, 8, true);
        t.set(id, 0, 7, false);
        assert_eq!(t.get(id, 0), Some(true));
        t.set(id, 0, 8, false);
        assert_eq!(t.get(id, 0), Some(false), "같은 seq 는 받는다");
    }

    #[test]
    fn writes_without_the_matching_registration_are_dropped() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.set(id, 0, 1, true);
        assert_eq!(t.get(id, 0), None, "등록 없는 쓰기는 자리를 만들지 않는다");
        assert_eq!(t.len(), 0);

        t.register(id, 5);
        t.set(id, 9, 1, true);
        assert_eq!(
            t.get(id, 5),
            Some(false),
            "옛 표식의 쓰기는 산 화신을 덮지 않는다"
        );
        assert_eq!(t.get(id, 9), None);
    }

    #[test]
    fn forget_removes_only_the_matching_incarnation_and_late_writes_stay_out() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.register(id, 1);
        t.set(id, 1, 1, true);
        t.forget(id, 0);
        assert_eq!(
            t.get(id, 1),
            Some(true),
            "지각 정리가 산 화신을 지우지 않는다"
        );
        t.forget(id, 1);
        t.set(id, 1, 2, true);
        assert_eq!(
            t.get(id, 1),
            None,
            "정리 뒤 늦은 쓰기가 항목을 되살리지 않는다"
        );
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn registration_replaces_the_previous_incarnation() {
        let t = InputsPendingTable::new();
        let id = AgentId::new_v4();
        t.register(id, 1);
        t.set(id, 1, 5, true);
        t.register(id, 2);
        assert_eq!(t.get(id, 1), None);
        assert_eq!(t.get(id, 2), Some(false));
        t.set(id, 2, 0, true);
        assert_eq!(
            t.get(id, 2),
            Some(true),
            "새 화신의 seq 는 0 부터 — 옛 seq 와 비교하지 않는다"
        );
        assert_eq!(t.len(), 1);
    }
}
