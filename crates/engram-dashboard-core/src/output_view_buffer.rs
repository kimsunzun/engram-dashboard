//! 출력 평면 재설계(ADR-0040) 1단계 — core 순수 자료구조 2조각(Tauri 무관, headless 단독 테스트).
//!
//! ## 왜 core 순수인가 (분리 경계 — ADR-0012)
//! src-tauri `AgentBufferStore`(다음 단계)는 `tauri::ipc::Channel`·`Arc<Mutex>` 동기화를 들어 Tauri 에
//! 묶이지만, "에이전트당 공유 콘텐츠 ring 을 cursor 부터 어떻게 읽나"·"슬롯별 진도를 어떻게 추적/리셋하나"
//! 같은 **자료구조·전이는 Tauri 와 무관한 순수부**다. 그 순수부만 여기로 떼어
//! `cargo test -p engram-dashboard-core` 로 단독 회귀한다(src-tauri lib test 는 WebView2 DLL 링크로
//! 실행 자체가 막혀 같은 crate 순수부도 못 돈다 — 그 회피).
//!
//! ## 두 조각 (TRD §1·모듈경계 — FIX 5: 공통 추출은 데이터 구조만)
//! - [`BoundedSeqLog`] = **콘텐츠, 에이전트당 공유 1벌**. 데몬 `ReplayBuffer`(`output_core.rs`)와 동형 —
//!   `VecDeque<Chunk>` + `total_bytes` + 이중상한(2MB OR 4096) `pop_front` evict. 데몬 struct 시그니처·
//!   동기화·C4 호출 구조는 **비공유**(데몬/클라 각자 소유) — 데이터 구조만 미러한다.
//! - [`SlotCursorMap`] = **보는 단위(slotId) 1차 키 → `ViewCursor{agent_id, cursor}`**. 같은 에이전트를
//!   N slot 이 보면 cursor N개 + 콘텐츠 1벌(상위 store 가 조립). epoch 은 이 구조가 직접 안 가짐
//!   (상위 store 가 태그) — 단 `reset()`/`reset_cursors_for_agent` 로 새 스트림 전환을 흡수한다.
//!
//! 두 구조 모두 **Tauri 무관**: `SlotId` 는 generic 파라미터(core 에 Tauri 타입 누출 0). 저장 단위 =
//! **원본 binary frame bytes**(seq 는 외부=데몬이 부여, 이 구조는 받은 seq 를 저장만).

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::hash::Hash;

use crate::agent::types::AgentId;

// ── BoundedSeqLog — 콘텐츠 공유 ring (에이전트당 1벌) ──────────────────────────────────────

/// 공유 ring 한 칸. `seq` 는 외부(데몬)가 부여하고 이 구조는 받은 값을 저장만 한다(자체 발급 안 함).
/// `bytes` = 원본 binary frame bytes(ANSI/부분 UTF-8/CR 보존 — 화면 동일성, TRD §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub seq: u64,
    pub bytes: Vec<u8>,
}

/// [`BoundedSeqLog::read_from`] 결과 분류 — 데몬 `subscribe_from` 의 `ReplayKind` 분기와 동형이되,
/// 여기는 cursor 한 축만 본다(epoch 은 상위 store 가 태그).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// cursor 이후(혹은 `None`=전체)의 tail 을 무손실로 반환. 보낼 게 없을 수도 있다
    /// (`Some(s)` 이고 `s >= latest` → 빈 결과 + `UpToDate`).
    Resumed,
    /// `Some(s)` 이고 `s < oldest`(읽은 seq 다음 칸이 버퍼 oldest 보다 앞 = gap) → 그 사이는 이미
    /// evict 됨(불가피 유실). oldest 부터 반환 + 잘림 신호. (`s == oldest-1` 즉 `seq>s` 의 첫 칸이
    /// 정확히 oldest 이면 빈틈 0 → `Resumed`.)
    Truncated,
    /// `Some(s)` 이고 `s >= latest`(이미 최신) 또는 버퍼가 비어 보낼 게 없음.
    UpToDate,
}

/// 에이전트당 공유 출력 ring — 상한 2MB **그리고** event 수 상한 4096.
///
/// ★이중상한 이유(데몬 `ReplayBuffer` 와 동일 근거)★: byte 상한만 있으면 1바이트 청크가 폭주할 때
/// event 수가 수백만으로 불어 신규 read 가 mpsc/Channel 을 즉시 가득 채워 매 재연결이 slow-consumer 로
/// 끊기는 영구 루프가 생긴다. 둘 중 하나라도 초과하면 앞부터 evict(oldest 상승).
///
/// 상수(2MB / 4096)는 데몬 `ReplayBuffer::new()` 와 정확히 일치시킨다 — 클라 보관 하한이 데몬 미러보다
/// 작으면 데몬엔 있는데 클라가 잘라낸 구간을 데몬 `after_seq`(버퍼 최신) 로 다시 못 받는 모순이 생긴다.
#[derive(Debug, Clone)]
pub struct BoundedSeqLog {
    chunks: VecDeque<Chunk>,
    total_bytes: usize,
    max_bytes: usize,
    max_events: usize,
}

impl BoundedSeqLog {
    pub fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            total_bytes: 0,
            // 데몬 ReplayBuffer 와 동일 상수 — 클라 보관 하한이 데몬 미러와 어긋나면 재구독 모순.
            max_bytes: 2 * 1024 * 1024,
            max_events: 4096,
        }
    }

    /// 외부(데몬)가 부여한 `seq` 의 청크를 append. 이중상한 초과 시 앞부터 evict(oldest 상승).
    ///
    /// seq 단조 증가는 호출자(데몬 스트림) 계약이다 — 이 구조는 강제하지 않고 받은 순서대로 저장만 한다
    /// (`read_from` 의 `partition_point` 가 seq 오름차순을 전제하므로 호출자가 단조를 깨면 안 됨).
    ///
    /// ★단일 append > max_bytes 보호(FIX-3, load-bearing)★: append 한 건이 byte 상한보다 커도
    /// **방금 넣은 마지막 1개는 절대 evict 하지 않는다**(`chunks.len() > 1` 가드). 데몬 `ReplayBuffer::push`
    /// 는 이 가드가 없어 거대 단일 chunk 가 들어오면 방금 넣은 것까지 비워 log 가 빈 상태가 되고, 이후
    /// `read_from` 이 잘림 신호 없이 `UpToDate` 를 줘 침묵 유실이 난다. view buffer 의 핵심 가치 = 무손실
    /// 이라, 상한을 넘더라도 마지막 1개를 보존해 그 chunk 가 read 로 반드시 나가게 한다(상한 일시 초과 허용 —
    /// 다음 append 가 들어오면 그때 정상 evict 된다). event 수 상한은 항상 ≥1 이라 동일 가드로 무해하다.
    pub fn append(&mut self, seq: u64, bytes: Vec<u8>) {
        self.total_bytes += bytes.len();
        self.chunks.push_back(Chunk { seq, bytes });
        // byte 상한 OR event 수 상한 둘 중 하나라도 넘으면 앞부터 제거.
        // 단, 방금 넣은 마지막 1개는 보존(len > 1 가드) — 거대 단일 chunk 의 침묵 유실 방지(FIX-3).
        while self.chunks.len() > 1
            && (self.total_bytes > self.max_bytes || self.chunks.len() > self.max_events)
        {
            if let Some(oldest) = self.chunks.pop_front() {
                self.total_bytes -= oldest.bytes.len();
            } else {
                break;
            }
        }
    }

    /// 버퍼에 남은 가장 오래된 seq. 비었으면 `None`.
    pub fn oldest_seq(&self) -> Option<u64> {
        self.chunks.front().map(|c| c.seq)
    }

    /// 버퍼에 남은 가장 최신 seq. 비었으면 `None`. (데몬 재구독 `after_seq` = 이 값, TRD §3 축 A.)
    pub fn latest_seq(&self) -> Option<u64> {
        self.chunks.back().map(|c| c.seq)
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// 누적 바이트(이중상한 검증·메모리 회계용).
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// `cursor` **이후**의 청크 슬라이스를 반환. 데몬 `subscribe_from(after_seq: Option<u64>)` 분기와
    /// 동형(FIX-1) — cursor 한 축만 본다(epoch 은 상위 store 가 태그).
    ///
    /// ★cursor 의미 (FIX-1 재정의, 데몬 `after_seq` 와 동형)★:
    /// - `None` = **아직 아무것도 안 읽음** → oldest 부터 **전체**(seq 0 포함). fresh mount 가 이 값.
    /// - `Some(s)` = seq `s` 까지 읽음 → `seq > s` tail.
    ///
    /// 재정의 이유(근원 결함): cursor 가 `u64` 였을 때 "아직 안 읽음=전체"를 표현할 방법이 없어
    /// (`read_from(0)` 은 seq 0 을 제외), seq 를 0 부터 발급하는 데몬에서 fresh mount 가 첫 출력을 영구
    /// 유실했다. `Option` 으로 oldest-1 / 0-1 underflow 도 구조적으로 제거된다.
    ///
    /// 계약:
    /// - 버퍼가 비었으면 → `([], UpToDate)`.
    /// - `None` → `Resumed`: oldest 부터 전체(seq 0 포함). 잘림 아님(처음 보는 거라 잃은 게 없음).
    /// - `Some(s)` 이고 `s >= latest` → `UpToDate`: 이미 최신(빈 슬라이스).
    /// - `Some(s)` 이고 `s + 1 < oldest`(읽은 다음 칸 `s+1` 이 버퍼 oldest 보다도 앞 = **gap**) →
    ///   `Truncated`: oldest 부터 전체(그 사이 evict 유실은 불가피, 잘림 신호).
    /// - 그 사이(`oldest-1 <= s < latest`, gap 없음) → `Resumed`: `seq <= s` prefix 를 건너뛴 tail.
    ///
    /// ★Truncated 경계가 데몬 `subscribe_from`(`s < oldest`)보다 한 칸 더 엄격(`s+1 < oldest`)한
    /// 이유(load-bearing)★: view cursor 는 numeric 단일 축이라, clamp(FIX-2)가 cursor 를
    /// `Some(oldest-1)` 로 맞춘다. 데몬 규칙(`s < oldest`)을 그대로 쓰면 `oldest-1 < oldest` 라 clamp
    /// 직후(빈틈 0인데도) 매번 false `Truncated` 가 난다. `s+1 < oldest`(읽은 다음 칸이 oldest 보다도
    /// 앞 = 진짜 유실)만 Truncated 로 보면, `s == oldest-1`(딱 oldest 직전, 빈틈 0)은 정상 `Resumed`.
    /// `saturating_add` 로 `s=u64::MAX` underflow 방어(그 경우 위 `s>=latest` 로 이미 반환되나 안전).
    /// 두 구조는 비공유(TRD 모듈경계)라 이 경계 정밀화는 데몬에 영향 없다.
    ///
    /// ★FIX-2 (clamp off-by-one) 와의 정합★: gap 복구 시 상위 store 는 뒤처진 cursor 를
    /// `clamp_cursors_for_agent(new_oldest)` 로 끌어올리는데, 그 clamp 가 cursor 를 `Some(new_oldest)`
    /// 가 아니라 **"new_oldest 를 포함하도록"** 맞춘다(= `new_oldest > 0` 면 `Some(new_oldest-1)`,
    /// `new_oldest==0` 이면 `None`). 그래야 clamp 직후 `read_from` 이 `seq > new_oldest-1` = new_oldest
    /// 부터 반환해 복구 후 첫 출력(new_oldest)을 무손실로 준다. (cursor 가 `Some(new_oldest)` 면
    /// `seq > new_oldest` 라 new_oldest 자체를 건너뛰는 것이 근원 결함이었다.)
    ///
    /// `&mut self` 인 이유(load-bearing): `VecDeque` 는 evict(`pop_front`) 누적 시 내부적으로 두 조각
    /// (wrap)으로 갈라져 `&self` 만으론 연속 슬라이스 1개를 못 준다. 진입 시 `make_contiguous` 로 1회
    /// 정규화해야 tail 슬라이스가 두 조각에 걸쳐도 안전하다. 상위 store 가 `Mutex`(`&mut`) 보유 중
    /// 호출하므로 `&mut` 요구는 무비용 — 호출자는 반환 슬라이스를 snapshot(`.to_vec()`) 떠 락 밖 send
    /// 한다(TRD §1 락 규율: 락 안에선 데이터 수집만, send 는 락 밖).
    pub fn read_from(&mut self, cursor: Option<u64>) -> (&[Chunk], ReadOutcome) {
        let (oldest, latest) = match (self.chunks.front(), self.chunks.back()) {
            (Some(f), Some(b)) => (f.seq, b.seq),
            _ => return (&[], ReadOutcome::UpToDate), // 빈 버퍼.
        };

        let (start_idx, outcome) = match cursor {
            // 아직 안 읽음 → oldest 부터 전체(seq 0 포함). 잃은 게 없으니 Resumed.
            None => (0usize, ReadOutcome::Resumed),
            Some(s) if s >= latest => return (&[], ReadOutcome::UpToDate), // 이미 최신.
            // gap: 읽은 다음 칸(s+1)이 버퍼 oldest 보다도 앞 = 그 사이 evict 유실. oldest 부터 전체.
            // saturating_add: s=u64::MAX underflow 방어(그 경우 위 s>=latest 로 이미 반환되나 안전).
            Some(s) if s.saturating_add(1) < oldest => (0usize, ReadOutcome::Truncated),
            // gap 없음 → seq <= s prefix 를 건너뛴 tail(seq 오름차순 전제 → partition_point 안전).
            Some(s) => {
                let idx = self.chunks.partition_point(|c| c.seq <= s);
                (idx, ReadOutcome::Resumed)
            }
        };

        // wrap 정규화 — 두 조각을 1개 연속 슬라이스로 합친 뒤 [start_idx..] 반환.
        let contiguous = self.chunks.make_contiguous();
        (&contiguous[start_idx..], outcome)
    }

    /// 전체 비우기(새 스트림 전환용 — epoch 리셋은 상위 store 가 태그하지만, 콘텐츠 reset 은 여기 제공).
    pub fn reset(&mut self) {
        self.chunks.clear();
        self.total_bytes = 0;
    }
}

impl Default for BoundedSeqLog {
    fn default() -> Self {
        Self::new()
    }
}

// ── SlotCursorMap — 슬롯별 진도(보는 단위가 1차 키) ──────────────────────────────────────

/// 한 슬롯이 어느 에이전트를 어디까지 봤는지. cursor = "마지막으로 읽은 seq"
/// ([`BoundedSeqLog::read_from`] 와 같은 의미). `None` = 아직 안 읽음(fresh mount = 전체 replay 대상),
/// `Some(s)` = seq `s` 까지 읽음(`seq > s` 가 미읽음). 데몬 `after_seq: Option<u64>` 와 동형(FIX-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewCursor {
    pub agent_id: AgentId,
    pub cursor: Option<u64>,
}

/// 슬롯(보는 단위) → `ViewCursor`. `SlotId` 는 generic(core 에 Tauri 타입 누출 0) — src-tauri 가
/// 실제 슬롯 식별자(WindowLabel·leaf id 등)를 박아 인스턴스화한다.
///
/// 콘텐츠는 여기 없다 — 에이전트당 1벌의 [`BoundedSeqLog`] 는 상위 store(`AgentBufferStore`)가 들고,
/// 이 맵은 "누가 어디까지 봤나"만 든다(같은 에이전트를 N slot 이 보면 cursor N개 + 콘텐츠 1벌).
#[derive(Debug, Clone)]
pub struct SlotCursorMap<S> {
    cursors: HashMap<S, ViewCursor>,
}

impl<S> SlotCursorMap<S>
where
    S: Eq + Hash + Clone,
{
    pub fn new() -> Self {
        Self {
            cursors: HashMap::new(),
        }
    }

    /// 슬롯에 에이전트를 배정 + 시작 cursor 기록. `start_cursor`:
    /// - `None` = 새 창 fresh mount = oldest 부터 전체 replay(가장 흔한 케이스).
    /// - `Some(s)` = 재구독 등 이미 seq `s` 까지 본 슬롯(이어보기).
    ///
    /// 이미 있던 슬롯이면 덮어쓴다(재배정).
    pub fn insert(&mut self, slot: S, agent_id: AgentId, start_cursor: Option<u64>) {
        self.cursors.insert(
            slot,
            ViewCursor {
                agent_id,
                cursor: start_cursor,
            },
        );
    }

    /// 슬롯 배정 해제(창 닫힘/재배정). 제거된 에이전트 id 반환(없던 슬롯이면 `None`).
    /// 호출자는 이 반환 + `agent_has_viewers` 로 "마지막 viewer 가 빠졌으니 콘텐츠 버퍼 폐기" 판정한다
    /// (TRD §4: cursor 0개면 그 에이전트 버퍼 drop).
    pub fn remove(&mut self, slot: &S) -> Option<AgentId> {
        self.cursors.remove(slot).map(|vc| vc.agent_id)
    }

    /// 슬롯 cursor 를 seq `new_cursor` 까지 전진. ★단조성★: 기존보다 작거나 같으면 무시(후퇴 없음 —
    /// 옛 낮은 seq 로 되돌아가면 이미 읽은 구간을 중복 전송). 기존이 `None`(아직 안 읽음)이면 어떤
    /// `new_cursor` 든 전진(최초 read 완료). 슬롯이 없으면 아무것도 안 함. 실제 전진했으면 `true`.
    pub fn advance(&mut self, slot: &S, new_cursor: u64) -> bool {
        if let Some(vc) = self.cursors.get_mut(slot) {
            // None < Some(any): 아직 안 읽음에서 처음 읽으면 무조건 전진. Some(a)->Some(b)는 b>a 만.
            let advance = match vc.cursor {
                None => true,
                Some(cur) => new_cursor > cur,
            };
            if advance {
                vc.cursor = Some(new_cursor);
                return true;
            }
        }
        false
    }

    /// 슬롯의 현재 `ViewCursor`(테스트·계산용).
    pub fn get(&self, slot: &S) -> Option<ViewCursor> {
        self.cursors.get(slot).copied()
    }

    /// 이 에이전트를 보는 슬롯들(fan-out 역조회 — agent 출력 도착 시 어느 슬롯에 보낼지).
    pub fn slots_for_agent(&self, agent_id: AgentId) -> impl Iterator<Item = &S> {
        self.cursors
            .iter()
            .filter(move |(_, vc)| vc.agent_id == agent_id)
            .map(|(slot, _)| slot)
    }

    /// 현재 cursor 를 든 모든 슬롯(agent 무관 — FIX-1 고아 sweep: router 에 없는 slot 전부 색출용).
    pub fn all_slots(&self) -> impl Iterator<Item = &S> {
        self.cursors.keys()
    }

    /// 이 에이전트를 보는 슬롯이 1개라도 있나(0개면 콘텐츠 버퍼 폐기 판정, TRD §4).
    pub fn agent_has_viewers(&self, agent_id: AgentId) -> bool {
        self.cursors.values().any(|vc| vc.agent_id == agent_id)
    }

    /// 이 에이전트를 보는 모든 슬롯의 cursor 를 `new_cursor` 로 리셋(epoch 전환 시). 새 스트림은 seq 를
    /// 0 부터 다시 발급하므로 보통 `None`(아직 안 읽음 = 새 스트림 전체 replay)을 넘긴다.
    /// 다른 에이전트를 보는 슬롯은 건드리지 않는다(TRD §4b: 새 스트림 seq 0 → 그 agent 창만 리셋).
    /// 리셋된 슬롯 수 반환.
    pub fn reset_cursors_for_agent(&mut self, agent_id: AgentId, new_cursor: Option<u64>) -> usize {
        let mut n = 0;
        for vc in self.cursors.values_mut() {
            if vc.agent_id == agent_id {
                vc.cursor = new_cursor;
                n += 1;
            }
        }
        n
    }

    /// gap(Truncated) 시 클램프 — 버퍼 oldest 가 `new_oldest` 로 올라갔을 때, 이 에이전트의 뒤처진
    /// 슬롯들이 **복구 후 첫 read 에서 `new_oldest` 를 포함**하도록 cursor 를 끌어올린다.
    /// `reset_cursors_for_agent` 와 달리 **후퇴 없이** 하한만 올린다. 클램프된 슬롯 수 반환.
    ///
    /// ★FIX-2 (off-by-one) 핵심★: cursor 는 "마지막으로 읽은 seq" 라 `read_from(Some(s))` 는 `seq > s`
    /// 만 준다. 그래서 clamp 가 cursor 를 `Some(new_oldest)` 로 올리면 다음 read 가 `seq > new_oldest`
    /// 라 **new_oldest 자체(복구 후 첫 출력)를 건너뛴다** — 이게 근원 결함이었다. new_oldest 를 *포함*
    /// 시키려면 cursor 를 그 **직전**으로 맞춰야 한다:
    /// - `new_oldest > 0` → `Some(new_oldest - 1)` (read = `seq > new_oldest-1` = new_oldest 부터).
    /// - `new_oldest == 0` → `None` (read = oldest 부터 전체, seq 0 포함). `Option` 이라 underflow 없음.
    ///
    /// 클램프 대상(이미 충분히 뒤이거나 안 읽은 슬롯만 — 후퇴 금지):
    /// - cursor `None` → 이미 "oldest 부터 전체" 대상이라 clamp 불필요(skip).
    /// - cursor `Some(s)` 이고 `s < new_oldest - 1`(= read 시작점이 new_oldest 보다 앞) → 끌어올림.
    /// - cursor `Some(s)` 이고 `s >= new_oldest - 1`(= 이미 new_oldest 이상부터 read) → 그대로(skip).
    pub fn clamp_cursors_for_agent(&mut self, agent_id: AgentId, new_oldest: u64) -> usize {
        // new_oldest 를 포함하는 cursor 값(그 직전). new_oldest==0 이면 None(전체, underflow 회피).
        let clamp_to = new_oldest.checked_sub(1);
        let mut n = 0;
        for vc in self.cursors.values_mut() {
            if vc.agent_id != agent_id {
                continue;
            }
            // 이미 clamp_to 이상(또는 None=전체)이면 후퇴가 되므로 건드리지 않는다.
            let needs_clamp = match (vc.cursor, clamp_to) {
                (None, _) => false,          // 이미 oldest 부터 전체 — clamp 불필요.
                (Some(s), Some(t)) => s < t, // 시작점이 new_oldest 보다 앞 → 끌어올림.
                // new_oldest==0 이면 아무것도 evict 안 됐다는 뜻(gap 불가) → clamp 불필요.
                (Some(_), None) => false,
            };
            if needs_clamp {
                vc.cursor = clamp_to;
                n += 1;
            }
        }
        n
    }

    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }

    /// 슬롯 entry 직접 접근(상위 store 가 cursor 갱신 후 콘텐츠 read 를 한 락 안에서 조립할 때).
    pub fn entry(&mut self, slot: S) -> Entry<'_, S, ViewCursor> {
        self.cursors.entry(slot)
    }
}

impl<S> Default for SlotCursorMap<S>
where
    S: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(n: u128) -> AgentId {
        AgentId::from_u128(n)
    }

    /// read_from 반환 슬라이스의 seq 들을 뽑는다(검증 편의).
    fn seqs(slice: &[Chunk]) -> Vec<u64> {
        slice.iter().map(|c| c.seq).collect()
    }

    // ── BoundedSeqLog: append / read_from 범위 정확 ──────────────────────────────────────

    #[test]
    fn append_then_read_from_returns_correct_range() {
        let mut log = BoundedSeqLog::new();
        for seq in 0..5u64 {
            log.append(seq, vec![b'a' + seq as u8]);
        }
        assert_eq!(log.oldest_seq(), Some(0));
        assert_eq!(log.latest_seq(), Some(4));

        // cursor=Some(1) → seq>1 인 [2,3,4] 만 Resumed.
        let (slice, outcome) = log.read_from(Some(1));
        assert_eq!(outcome, ReadOutcome::Resumed);
        assert_eq!(seqs(slice), vec![2, 3, 4]);
        // 바이트도 정확히.
        assert_eq!(slice[0].bytes, vec![b'c']);
    }

    #[test]
    fn fresh_mount_none_cursor_returns_all_including_seq_zero() {
        // ★FIX-1 핵심★: cursor=None(아직 안 읽음) → oldest(=seq 0) 부터 전체. seq 0 이 포함돼야
        //   데몬이 seq 0 부터 발급할 때 fresh mount 가 첫 출력을 유실하지 않는다(u64 모델의 근원 결함).
        let mut log = BoundedSeqLog::new();
        log.append(0, b"x".to_vec());
        log.append(1, b"y".to_vec());
        log.append(2, b"z".to_vec());
        let (slice, outcome) = log.read_from(None);
        assert_eq!(outcome, ReadOutcome::Resumed);
        assert_eq!(seqs(slice), vec![0, 1, 2], "fresh mount 는 seq 0 포함 전체");
        assert_eq!(slice[0].bytes, b"x", "seq 0 의 바이트가 정확히 첫 칸");
    }

    #[test]
    fn read_from_some_zero_excludes_seq_zero() {
        // cursor=Some(0) 의미 "seq 0 까지 읽음" → seq>0 인 [1,2] (seq 0 제외).
        // None(전체) 과의 구분 핀: Some(0) 은 "0 봤음", None 은 "아무것도 안 봤음".
        let mut log = BoundedSeqLog::new();
        log.append(0, b"x".to_vec());
        log.append(1, b"y".to_vec());
        log.append(2, b"z".to_vec());
        let (slice, outcome) = log.read_from(Some(0));
        assert_eq!(outcome, ReadOutcome::Resumed);
        assert_eq!(seqs(slice), vec![1, 2]);
    }

    #[test]
    fn fresh_mount_when_stream_starts_late_is_resumed() {
        // stream 이 seq 1 부터(oldest=1)여도 fresh mount(None)는 oldest 부터 전체 = [1,2,3] Resumed.
        // u64 모델 시절의 oldest-1 시작점 트릭이 사라져 false Truncated 경계 자체가 없어졌다.
        let mut log = BoundedSeqLog::new();
        for seq in 1..=3u64 {
            log.append(seq, b"x".to_vec());
        }
        assert_eq!(log.oldest_seq(), Some(1));
        let (slice, outcome) = log.read_from(None);
        assert_eq!(outcome, ReadOutcome::Resumed, "fresh mount → Resumed");
        assert_eq!(seqs(slice), vec![1, 2, 3], "oldest 부터 전체 무손실");
    }

    #[test]
    fn read_from_caught_up_is_empty_uptodate() {
        let mut log = BoundedSeqLog::new();
        for seq in 0..3u64 {
            log.append(seq, b"x".to_vec());
        }
        // cursor=Some(2)(=latest) → 보낼 게 없음.
        let (slice, outcome) = log.read_from(Some(2));
        assert_eq!(outcome, ReadOutcome::UpToDate);
        assert!(slice.is_empty());
        // cursor 가 latest 보다 더 커도 UpToDate(과거 epoch 잔재 등 방어).
        let (slice2, outcome2) = log.read_from(Some(99));
        assert_eq!(outcome2, ReadOutcome::UpToDate);
        assert!(slice2.is_empty());
    }

    #[test]
    fn read_from_empty_buffer_is_uptodate() {
        let mut log = BoundedSeqLog::new();
        // None(fresh)이든 Some 이든 빈 버퍼는 UpToDate.
        let (slice, outcome) = log.read_from(None);
        assert_eq!(outcome, ReadOutcome::UpToDate);
        assert!(slice.is_empty());
        let (slice2, outcome2) = log.read_from(Some(0));
        assert_eq!(outcome2, ReadOutcome::UpToDate);
        assert!(slice2.is_empty());
        assert_eq!(log.oldest_seq(), None);
        assert_eq!(log.latest_seq(), None);
    }

    // ── BoundedSeqLog: 이중상한 evict ────────────────────────────────────────────────────

    #[test]
    fn evicts_on_event_count_cap() {
        // 1바이트 청크 5000개 → byte 상한(2MB) 한참 밑이지만 event 상한(4096)에 걸려 evict.
        let mut log = BoundedSeqLog::new();
        for seq in 0..5000u64 {
            log.append(seq, vec![b'x']);
        }
        assert_eq!(log.len(), 4096);
        // 가장 오래된 것부터 evict → oldest = 5000-4096 = 904.
        assert_eq!(log.oldest_seq(), Some(904));
        assert_eq!(log.latest_seq(), Some(4999));
    }

    #[test]
    fn evicts_on_byte_cap() {
        // 큰 청크로 byte 상한(2MB) 초과 → event 수는 4096 미만이지만 byte 상한으로 evict.
        let mut log = BoundedSeqLog::new();
        let chunk = vec![0u8; 256 * 1024]; // 256KiB
                                           // 9개 × 256KiB = 2.25MiB > 2MiB → 최소 1개 evict.
        for seq in 0..9u64 {
            log.append(seq, chunk.clone());
        }
        assert!(log.total_bytes() <= 2 * 1024 * 1024);
        assert!(log.len() < 9, "byte 상한으로 적어도 1개는 evict");
        // oldest 상승 확인(0 이 빠짐).
        assert!(log.oldest_seq().unwrap() >= 1);
    }

    #[test]
    fn evicted_cursor_below_oldest_is_truncated() {
        let mut log = BoundedSeqLog::new();
        for seq in 0..5000u64 {
            log.append(seq, vec![b'x']);
        }
        // oldest=904. cursor=Some(10) < oldest → Truncated, oldest 부터 전체.
        let (slice, outcome) = log.read_from(Some(10));
        assert_eq!(outcome, ReadOutcome::Truncated);
        assert_eq!(seqs(slice).first().copied(), Some(904));
        assert_eq!(seqs(slice).last().copied(), Some(4999));
        assert_eq!(slice.len(), 4096);
    }

    #[test]
    fn cursor_at_oldest_minus_one_is_resumed_includes_oldest() {
        // 경계 핀: oldest=904 일 때 cursor=Some(903)(=oldest-1) → 빈틈 0 → Resumed, oldest(904) 포함.
        // FIX-2 의 clamp 결과(Some(new_oldest-1))가 new_oldest 를 무손실로 주는 것과 같은 경계.
        let mut log = BoundedSeqLog::new();
        for seq in 0..5000u64 {
            log.append(seq, vec![b'x']);
        }
        assert_eq!(log.oldest_seq(), Some(904));
        let (slice, outcome) = log.read_from(Some(903));
        assert_eq!(outcome, ReadOutcome::Resumed, "oldest-1 = 빈틈 0 → Resumed");
        assert_eq!(seqs(slice).first().copied(), Some(904), "oldest 포함");
        assert_eq!(slice.len(), 4096);
    }

    #[test]
    fn read_from_after_eviction_still_contiguous() {
        // evict(pop_front) 가 VecDeque wrap 을 만들 수 있는데 make_contiguous 정규화로
        // tail 이 끊김 없이 나오는지 — wrap 경계를 넘는 cursor read.
        let mut log = BoundedSeqLog::new();
        // 4096 채운 뒤 추가로 더 넣어 여러 번 pop_front 발생(wrap 유발).
        for seq in 0..6000u64 {
            log.append(seq, vec![b'x']);
        }
        let oldest = log.oldest_seq().unwrap(); // 6000-4096 = 1904
        assert_eq!(oldest, 1904);
        // oldest 직후부터 끝까지 연속으로 나와야(개수 = latest-cursor).
        let (slice, outcome) = log.read_from(Some(2000));
        assert_eq!(outcome, ReadOutcome::Resumed);
        assert_eq!(seqs(slice).first().copied(), Some(2001));
        assert_eq!(seqs(slice).last().copied(), Some(5999));
        assert_eq!(slice.len(), (5999 - 2000) as usize);
    }

    // ── BoundedSeqLog: 단일 append > max_bytes 경계 (FIX-3) ──────────────────────────────

    #[test]
    fn single_append_over_byte_cap_is_preserved_not_silently_lost() {
        // ★FIX-3★: byte 상한(2MiB)보다 큰 단일 chunk 가 들어와도 방금 넣은 마지막 1개는 보존된다.
        //   (보호 가드 없으면 while evict 가 그것까지 비워 log 가 빈 상태 → read 가 UpToDate 로 침묵 유실.)
        let mut log = BoundedSeqLog::new();
        let huge = vec![0u8; 3 * 1024 * 1024]; // 3MiB > 2MiB 상한
        log.append(7, huge);
        // 상한을 일시 초과하더라도 그 chunk 는 버퍼에 남는다.
        assert_eq!(log.len(), 1, "단일 거대 chunk 는 보존(침묵 유실 금지)");
        assert_eq!(log.oldest_seq(), Some(7));
        assert!(
            log.total_bytes() > 2 * 1024 * 1024,
            "상한 일시 초과 허용(마지막 1개 보존)"
        );
        // fresh read(None)가 그 chunk 를 실제로 돌려줘야 한다 — UpToDate(침묵) 아님.
        let (slice, outcome) = log.read_from(None);
        assert_eq!(outcome, ReadOutcome::Resumed);
        assert_eq!(seqs(slice), vec![7], "거대 chunk 가 read 로 나옴");
    }

    #[test]
    fn append_after_oversized_chunk_evicts_it() {
        // 거대 chunk 가 일시 보존되더라도, 다음 append 가 들어오면 그때 정상 evict 되어
        // 상한 초과 상태가 지속되지 않는다(len>1 가드라 마지막 1개만 보존).
        let mut log = BoundedSeqLog::new();
        log.append(1, vec![0u8; 3 * 1024 * 1024]); // 3MiB
        log.append(2, vec![b'z']); // 작은 후속 → 거대 chunk evict.
        assert_eq!(log.len(), 1, "후속 append 가 거대 chunk 를 정상 evict");
        assert_eq!(log.oldest_seq(), Some(2));
        assert!(log.total_bytes() <= 2 * 1024 * 1024, "상한 회복");
    }

    // ── BoundedSeqLog: reset ─────────────────────────────────────────────────────────────

    #[test]
    fn reset_empties_buffer() {
        let mut log = BoundedSeqLog::new();
        for seq in 0..10u64 {
            log.append(seq, vec![b'x'; 100]);
        }
        log.reset();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.total_bytes(), 0);
        assert_eq!(log.oldest_seq(), None);
        let (slice, outcome) = log.read_from(None);
        assert_eq!(outcome, ReadOutcome::UpToDate);
        assert!(slice.is_empty());
    }

    // ── SlotCursorMap: insert / slots_for_agent fan-out ─────────────────────────────────

    #[test]
    fn n_slots_viewing_same_agent_listed() {
        let a = aid(1);
        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        map.insert(1, a, None);
        map.insert(2, a, None);
        map.insert(3, a, None);
        // 다른 agent 1개 — 역조회에 안 섞여야.
        map.insert(4, aid(2), None);

        let mut slots: Vec<u32> = map.slots_for_agent(a).copied().collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![1, 2, 3]);
        assert_eq!(map.len(), 4);
    }

    #[test]
    fn agent_has_viewers_false_after_last_slot_removed() {
        let a = aid(1);
        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        map.insert(1, a, None);
        map.insert(2, a, None);
        assert!(map.agent_has_viewers(a));

        // 첫 슬롯 제거 → 아직 viewer 있음.
        assert_eq!(map.remove(&1), Some(a));
        assert!(map.agent_has_viewers(a));

        // 마지막 슬롯 제거 → viewer 0개 전이(콘텐츠 폐기 판정 트리거).
        assert_eq!(map.remove(&2), Some(a));
        assert!(!map.agent_has_viewers(a));

        // 없던 슬롯 제거 → None.
        assert_eq!(map.remove(&99), None);
    }

    #[test]
    fn advance_is_monotonic() {
        let a = aid(1);
        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        map.insert(1, a, Some(5));

        // 전진.
        assert!(map.advance(&1, 10));
        assert_eq!(map.get(&1).unwrap().cursor, Some(10));
        // 후퇴 시도는 무시(단조성).
        assert!(!map.advance(&1, 3));
        assert_eq!(map.get(&1).unwrap().cursor, Some(10));
        // 같은 값도 전진 아님.
        assert!(!map.advance(&1, 10));
        // 없는 슬롯 전진은 false.
        assert!(!map.advance(&99, 100));
    }

    #[test]
    fn advance_from_none_always_progresses() {
        // None(아직 안 읽음)에서 처음 read 완료 시 어떤 seq 든 전진(None < Some(any)).
        let a = aid(1);
        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        map.insert(1, a, None);
        assert!(
            map.advance(&1, 0),
            "None→Some(0) 도 전진(seq 0 까지 읽음 기록)"
        );
        assert_eq!(map.get(&1).unwrap().cursor, Some(0));
        // 이후 단조.
        assert!(!map.advance(&1, 0));
        assert!(map.advance(&1, 5));
        assert_eq!(map.get(&1).unwrap().cursor, Some(5));
    }

    #[test]
    fn reset_cursors_only_for_target_agent() {
        let a = aid(1);
        let b = aid(2);
        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        map.insert(1, a, Some(100));
        map.insert(2, a, Some(50));
        map.insert(3, b, Some(70)); // 다른 agent — 보존돼야.

        // epoch 전환 → 새 스트림 전체 replay 대상으로 None 리셋.
        let n = map.reset_cursors_for_agent(a, None);
        assert_eq!(n, 2, "agent a 슬롯 2개만 리셋");
        assert_eq!(map.get(&1).unwrap().cursor, None);
        assert_eq!(map.get(&2).unwrap().cursor, None);
        assert_eq!(map.get(&3).unwrap().cursor, Some(70), "다른 agent 보존");
    }

    #[test]
    fn clamp_cursors_only_raises_below_oldest() {
        let a = aid(1);
        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        // new_oldest=50 → clamp 목표 = Some(49)(50 직전, read 가 50 포함).
        map.insert(1, a, Some(100)); // 100 >= 49 → 그대로(후퇴 금지).
        map.insert(2, a, Some(30)); // 30 < 49 → Some(49) 로 클램프.
        map.insert(3, aid(2), Some(10)); // 다른 agent → 무관.

        let n = map.clamp_cursors_for_agent(a, 50);
        assert_eq!(n, 1, "cursor < new_oldest-1 인 agent a 슬롯 1개만 클램프");
        assert_eq!(
            map.get(&1).unwrap().cursor,
            Some(100),
            "이미 충분히 뒤 → 후퇴 없음"
        );
        assert_eq!(
            map.get(&2).unwrap().cursor,
            Some(49),
            "new_oldest 포함하도록 그 직전(49)으로 클램프(FIX-2)"
        );
        assert_eq!(map.get(&3).unwrap().cursor, Some(10), "다른 agent 보존");
    }

    #[test]
    fn clamp_then_read_includes_new_oldest() {
        // ★FIX-4 (FIX-2 회귀 가드)★: clamp 후 read_from 이 **new_oldest 를 실제로 포함**하는지 단언.
        //   (기존 테스트는 cursor 값만 봤다 — off-by-one 이 read 결과로 새는지 못 잡았다.)
        let a = aid(1);
        let mut log = BoundedSeqLog::new();
        for seq in 0..5000u64 {
            log.append(seq, vec![b'x']);
        }
        let new_oldest = log.oldest_seq().unwrap(); // 904
        assert_eq!(new_oldest, 904);

        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        map.insert(1, a, Some(10)); // 뒤처진 슬롯(이미 evict 된 구간을 가리킴).

        let n = map.clamp_cursors_for_agent(a, new_oldest);
        assert_eq!(n, 1);
        // clamp 후 그 슬롯 cursor 로 read → new_oldest(904) 가 **첫 칸으로 포함**돼야 한다.
        let cur = map.get(&1).unwrap().cursor;
        let (slice, outcome) = log.read_from(cur);
        assert_eq!(outcome, ReadOutcome::Resumed, "clamp 후엔 gap 0 → Resumed");
        assert_eq!(
            seqs(slice).first().copied(),
            Some(new_oldest),
            "복구 후 첫 출력(new_oldest)이 유실되지 않고 포함"
        );
        assert_eq!(slice.len(), 4096, "new_oldest~latest 전체 무손실");
    }

    #[test]
    fn clamp_at_oldest_zero_is_noop() {
        // new_oldest=0 이면 evict 가 없었다는 뜻(gap 불가) → Some(s) 슬롯을 건드리지 않는다.
        let a = aid(1);
        let mut map: SlotCursorMap<u32> = SlotCursorMap::new();
        map.insert(1, a, Some(3));
        map.insert(2, a, None);
        let n = map.clamp_cursors_for_agent(a, 0);
        assert_eq!(n, 0, "new_oldest=0 → clamp 대상 없음");
        assert_eq!(map.get(&1).unwrap().cursor, Some(3));
        assert_eq!(map.get(&2).unwrap().cursor, None);
    }

    // ── 멀티뷰 시나리오: 공유 콘텐츠 1벌 + per-slot cursor 독립 read ──────────────────────

    #[test]
    fn multiview_independent_cursors_read_shared_content() {
        // agent A 를 slot1/2/3 이 본다. 콘텐츠 1벌(BoundedSeqLog), cursor 3개(SlotCursorMap).
        let a = aid(1);
        let mut content = BoundedSeqLog::new();

        // A 가 5건 append(seq 1..=5 — 외부 부여, 스트림 첫 seq=1).
        for seq in 1..=5u64 {
            content.append(seq, vec![b'0' + seq as u8]);
        }

        // ★새 mount cursor = None★ — 아직 아무것도 안 읽은 fresh mount 는 None 으로 배정하면
        //   read_from(None)이 oldest 부터 전체를 준다(stream 이 seq 1 부터 늦게 시작해도 무손실).
        //   상위 store 는 배정 시 그냥 None 을 넣는다(oldest-1 underflow 트릭 불필요).
        let mut cursors: SlotCursorMap<u32> = SlotCursorMap::new();
        cursors.insert(1, a, None);
        cursors.insert(2, a, None);
        cursors.insert(3, a, None);

        // 각 slot 이 자기 cursor(None)부터 read → 셋 다 [1,2,3,4,5] 무손실(eviction 없으니 Resumed).
        for slot in [1u32, 2, 3] {
            let cur = cursors.get(&slot).unwrap().cursor;
            let (slice, outcome) = content.read_from(cur);
            assert_eq!(
                outcome,
                ReadOutcome::Resumed,
                "slot{slot}: 미잘림 → Resumed"
            );
            assert_eq!(seqs(slice), vec![1, 2, 3, 4, 5], "slot{slot} 독립 무손실");
        }

        // slot1/3 은 5까지 따라잡고, slot2 만 seq 2 에서 뒤처진 상태로 둔다.
        cursors.advance(&1, 5);
        cursors.advance(&3, 5);
        cursors.advance(&2, 2);

        // 추가 append(6,7).
        content.append(6, b"f".to_vec());
        content.append(7, b"g".to_vec());

        // slot1: cursor=Some(5) → [6,7] 만.
        let (s1, o1) = content.read_from(cursors.get(&1).unwrap().cursor);
        assert_eq!(o1, ReadOutcome::Resumed);
        assert_eq!(seqs(s1), vec![6, 7]);

        // slot2: cursor=Some(2)(뒤처짐) → [3,4,5,6,7] 정확한 구간.
        let (s2, o2) = content.read_from(cursors.get(&2).unwrap().cursor);
        assert_eq!(o2, ReadOutcome::Resumed);
        assert_eq!(seqs(s2), vec![3, 4, 5, 6, 7]);

        // slot3: cursor=Some(5) → [6,7].
        let (s3, _) = content.read_from(cursors.get(&3).unwrap().cursor);
        assert_eq!(seqs(s3), vec![6, 7]);
    }

    #[test]
    fn fresh_mount_from_seq_zero_stream_is_lossless() {
        // ★FIX-1 통합 시나리오★: 데몬이 seq 0 부터 발급하는 스트림에 fresh mount(None)가 붙으면
        //   seq 0(첫 출력)을 포함해 전체를 무손실로 받아야 한다(u64 모델의 근원 결함 회귀 가드).
        let a = aid(1);
        let mut content = BoundedSeqLog::new();
        for seq in 0..4u64 {
            content.append(seq, vec![b'a' + seq as u8]);
        }
        let mut cursors: SlotCursorMap<u32> = SlotCursorMap::new();
        cursors.insert(1, a, None); // fresh mount.

        let (slice, outcome) = content.read_from(cursors.get(&1).unwrap().cursor);
        assert_eq!(outcome, ReadOutcome::Resumed);
        assert_eq!(seqs(slice), vec![0, 1, 2, 3], "seq 0 포함 전체 무손실");
        assert_eq!(slice[0].bytes, b"a", "seq 0 의 첫 출력 바이트 보존");
    }
}
