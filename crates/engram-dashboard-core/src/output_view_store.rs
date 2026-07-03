//! 출력 평면 재설계(ADR-0040) 2단계 — core 순수 조립 struct(Tauri 무관, headless 단독 테스트).
//!
//! ## 무엇인가 (1단계 두 조각의 조립부 — 사용자 확정)
//! 1단계가 만든 두 순수 자료구조([`BoundedSeqLog`](crate::output_view_buffer::BoundedSeqLog) =
//! 에이전트당 공유 콘텐츠 ring, [`SlotCursorMap`](crate::output_view_buffer::SlotCursorMap) = 슬롯별
//! 진도)를 **에이전트당 콘텐츠 1벌 + 슬롯별 cursor + per-agent epoch 태그**로 조립한 순수 store 다.
//! src-tauri `AgentBufferStore`(`Arc<Mutex<OutputViewStore<WindowLabel>>>`)가 이걸 Tauri 결합부
//! (`Channel`·`Mutex`)로 감싼다.
//!
//! ## ★Channel 을 들지 않는다 — snapshot 반환(이 분리의 전부)★
//! 이 store 는 Tauri `Channel`·실제 전송을 모른다. 모든 메서드는 "어느 slot 에 어떤 bytes 를
//! 보내야 하는지"를 `Vec<(S, Vec<u8>)>` **snapshot 으로 반환만** 한다 — 실제 Channel `send` 는
//! src-tauri 호출자가 **버퍼 락을 푼 뒤** 한다. 이게 TRD §1 락 규율(버퍼 락 안=데이터 수집만, send=
//! 락 밖)을 코드 구조로 강제하는 동시에, 조립 로직 전체를 `cargo test -p engram-dashboard-core` 로
//! headless 단독 회귀하게 한다(src-tauri lib test 는 WebView2 DLL 링크로 실행 자체가 막힌다 — 그 회피).
//!
//! ## 두 축 분리(TRD §3 — ★급소, /review code deep 대상)
//! - **축 A(데몬↔클라 동기화):** 데몬 재구독 `after_seq` = 버퍼 최신 seq([`resubscribe_after_seq`]).
//!   데몬은 클라에 없는 것만 보내 append 한다(비중복). `min_render_seq`(가장 뒤처진 창 합산) 모델은
//!   폐기 — 재구독 기준은 창이 아니라 버퍼 최신이다.
//! - **축 B(창별 read 무손실):** 각 slot 은 per-view cursor 로 공유 버퍼를 read([`on_frame`] 의
//!   slot 루프). 미렌더 창도 cursor 가 보존돼 재연결 후 무손실.
//! - 이 둘을 **절대 합치지 않는다** — 합치면 미렌더 창 유실/중복(초안 모순, opus·Codex 수렴 지적).
//!   [`OutputViewStore`] 는 cursor(축 B)만 들고, 축 A 는 `latest_seq` 한 줄로 분리돼 있다.
//!
//! ## epoch 태깅(TRD §4b — 락 아님)
//! per-agent epoch 태그(`epochs: HashMap<AgentId, u64>`)를 든다. [`on_frame`] 이 `frame.epoch ≠ 태그`면
//! 그 agent 콘텐츠 reset + 그 agent 의 모든 cursor reset 후 태그 갱신한다(SubscribeAck 를 기다리지
//! 않고 frame.epoch 기준 — single actor 직렬이라 락이 아니라 태깅으로 충분, ADR-0007). epoch 태깅은
//! **해당 agent 만** 건드린다(다른 agent 스트림 보존).
//!
//! ## ★cursor advance ⟺ Channel delivery 성공(근원2 FIX — /review code deep BLOCK)★
//! cursor 의 생명주기(router 파생 viewer 집합)와 Channel 의 생명주기(webview mount = `subscribe_output`
//! 의 registry insert)는 **다른 시점**이다 — layout 이 agent 를 배정해 router 에 (window,agent) slot 이
//! 생겨도, 그 창 webview 가 아직 mount 안 됐으면 Channel 은 registry 에 없다. 옛 구현은 [`on_frame`] 이
//! `sync_viewers`(router 기준)로 cursor 를 신설·advance 하고 그 snapshot 을 flush 했는데, flush 가
//! **registry miss 로 스킵하는 동안에도 cursor 는 이미 advance** 됐다 → 이후 [`subscribe`] 가 "cursor 가
//! 있다"고 불가침 판정해 빈 replay 를 줘 그 구간이 **영구 유실**된다(특히 webview reload 시 Channel 이
//! 교체되는데 stale cursor 가 남아 빈 화면).
//!
//! ★불변식★: **전달 성공한 slot 의 cursor 만 advance 한다.** core 는 Tauri registry 를 모르므로(ADR-0012),
//! src-tauri 가 "현재 Channel 이 등록된 slot 집합"을 [`on_frame`]·[`subscribe_agent`] 에 `deliverable`
//! 인자로 주입한다. deliverable 에 *없는* slot 은 **snapshot 에 안 담고 cursor advance 도 안 한다**(전달
//! 못 한 걸 전달한 것처럼 전진 금지). 그 slot 은 cursor 가 멈춰 있다가, 그 창 webview 가 mount 해
//! `subscribe_output` 이 cursor 를 fresh(None) 리셋 + replay 트리거할 때 **처음부터 무손실**로 받는다
//! (Channel 신규 등록 = viewer 재시작이라 stale 이어보기가 아니라 전체 replay 가 맞다 — reload 빈 화면
//! 차단). cursor 신설(`sync_viewers`/`subscribe_agent`)은 deliverable 무관하게 하되(라우팅 멤버십 추적은
//! frame 도착 시점이 정답), **advance 만 deliverable 게이트**를 통과시킨다 — 신설과 전진을 분리해 두 축
//! (멤버십 ↔ 진도)이 섞이지 않게 한다.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use crate::agent::types::AgentId;
use crate::output_view_buffer::{BoundedSeqLog, ReadOutcome, SlotCursorMap};

/// 에이전트당 공유 콘텐츠 + 슬롯별 cursor + per-agent epoch 태그를 조립한 순수 store.
///
/// `S` = 슬롯 키(generic — core 에 Tauri 타입 누출 0). src-tauri 가 실제 식별자(`WindowLabel` 등)를
/// 박아 인스턴스화한다.
///
/// ★소유권 한 락★(상위 store 가 `Mutex` 로 감쌈): `content`·`cursors`·`epochs` 를 **한 store 안**에
/// 둬 별도 맵 분리 시 생기는 락 순서 역전을 원천 제거한다(TRD §1).
#[derive(Debug)]
pub struct OutputViewStore<S> {
    /// 에이전트당 공유 콘텐츠 ring(1벌). 같은 agent 를 N slot 이 봐도 콘텐츠는 1벌 + cursor N개.
    content: HashMap<AgentId, BoundedSeqLog>,
    /// 슬롯(보는 단위) → `{agent_id, cursor}`. fan-out 역조회·진도 추적.
    cursors: SlotCursorMap<S>,
    /// per-agent epoch 태그. frame.epoch 와 비교해 새 스트림 전환(reset)을 판정한다(§4b).
    /// agent 마다 독립 스트림이라 per-agent — 한 agent epoch 전환이 다른 agent 를 건드리지 않는다.
    epochs: HashMap<AgentId, u64>,
}

impl<S> OutputViewStore<S>
where
    S: Eq + Hash + Clone,
{
    pub fn new() -> Self {
        Self {
            content: HashMap::new(),
            cursors: SlotCursorMap::new(),
            epochs: HashMap::new(),
        }
    }

    /// 데몬 binary frame 1건 도착 시: epoch 태깅 → append → 그 agent 를 보는 모든 slot 의 cursor 부터
    /// read → 보낼 `(slot, bytes)` snapshot 수집·cursor 전진. **반환을 락 밖에서 Channel send 한다.**
    ///
    /// 처리 순서(TRD §4b·§3 축 B·§2):
    /// - **(a) epoch 태깅:** 이 agent 태그 ≠ `frame_epoch` → 그 agent `content.reset()` +
    ///   `cursors.reset_cursors_for_agent(agent, None)`(모든 cursor 를 "처음부터 전체" 로) + 태그 갱신.
    ///   SubscribeAck 를 기다리지 않는다(frame.epoch 가 진실원 — `decide_epoch` 의 `st.epoch=None` 통과와
    ///   정합). 태깅은 **이 agent 만** 건드린다(다른 agent 스트림 보존).
    /// - **(b) append:** `content[agent].append(seq, bytes)`. 콘텐츠가 없으면 신설(빈 ring) 후 append.
    /// - **(c) fan-out read:** 그 agent 를 보는 slot 각각에 대해 `read_from(cursor)`:
    ///     - `Truncated`(gap) → `clamp_cursors_for_agent(agent, new_oldest)` 로 뒤처진 cursor 를
    ///       new_oldest 포함하도록 끌어올린 뒤 **그 slot 을 다시 read**(clamp 후엔 gap 0 → Resumed).
    ///     - 반환 슬라이스를 `.to_vec()` snapshot 으로 떠 결과에 담고, `cursors.advance(slot, last_seq)`.
    ///       (`UpToDate`/빈 슬라이스면 보낼 게 없어 snapshot 에 안 담고 advance 도 안 함.)
    ///
    /// ## ★`deliverable` 게이트(근원2 FIX — cursor advance ⟺ Channel delivery)★
    /// `deliverable` = 현재 **Channel 이 registry 에 등록된 slot 집합**(src-tauri 가 주입 — core 는 registry
    /// 무관). slot 이 deliverable 에 **없으면**(layout 이 배정했으나 그 창 webview 가 아직 mount 안 됨)
    /// 그 slot 은 read 슬라이스를 snapshot 에 **안 담고 cursor advance 도 건너뛴다** — flush 가 어차피
    /// registry miss 로 스킵할 구간을 "전달한 것처럼" advance 하면 그 출력이 영구 유실되기 때문이다
    /// (이후 subscribe 가 cursor 있다고 불가침 → 빈 replay). cursor 는 멈춰 있다가 그 창 mount 시
    /// `subscribe_output`(cursor fresh 리셋 + replay)이 처음부터 무손실로 채운다. ★epoch 태깅·append·
    /// 콘텐츠 신설은 deliverable 무관★(축 A 재구독 대비 콘텐츠는 항상 쌓이고, 멤버십 추적도 frame 시점이
    /// 정답) — deliverable 은 **advance/snapshot 만** 게이트한다(멤버십 ↔ 진도 분리).
    ///
    /// ★Truncated clamp 후 재read 가 안전한 이유★: 1단계 `clamp_cursors_for_agent` 는 cursor 를
    /// `new_oldest` 를 *포함*하도록(off-by-one 계약, FIX-2) 끌어올린다 → 재read 는 `Resumed` 로 oldest
    /// 부터 무손실. 잘림 미표시(사용자 결정 — TRD §3). ★clamp 는 deliverable 무관하게 한다★: gap 복구는
    /// "이미 evict 된 구간을 못 받음"의 표현이라 전달 가능 여부와 독립 — 안 그러면 미mount 창의 cursor 가
    /// 옛 evict 위치에 영영 묶여, mount 시 subscribe 가 fresh(None) 리셋하므로 무해(그땐 전체 replay).
    pub fn on_frame(
        &mut self,
        agent_id: AgentId,
        frame_epoch: u64,
        seq: u64,
        bytes: Vec<u8>,
        deliverable: &HashSet<S>,
    ) -> Vec<(S, Vec<u8>)> {
        // (a) epoch 태깅 — frame.epoch 와 태그 비교. 다르면 새 스트림 전환(이 agent 만 reset).
        //     ★최초 frame(태그 없음)도 여기서 태그를 박는다★: entry 없으면 insert 하고 reset 은 스킵
        //     (빈 콘텐츠라 reset 무의미, cursor 도 신설 시 None). 태그 ≠ frame_epoch 인 경우만 reset.
        match self.epochs.entry(agent_id) {
            Entry::Occupied(mut e) => {
                if *e.get() != frame_epoch {
                    // 새 스트림(epoch 전환) — 그 agent 콘텐츠/커서만 reset. 다른 agent 보존.
                    if let Some(log) = self.content.get_mut(&agent_id) {
                        log.reset();
                    }
                    self.cursors.reset_cursors_for_agent(agent_id, None);
                    e.insert(frame_epoch);
                }
            }
            Entry::Vacant(e) => {
                // 이 agent 의 첫 frame — 태그만 박는다(콘텐츠는 아래 (b) 가 신설, reset 불필요).
                e.insert(frame_epoch);
            }
        }

        // (b) append — 콘텐츠가 없으면 신설 후 append.
        let log = self.content.entry(agent_id).or_default();
        log.append(seq, bytes);

        // (c) fan-out read — 그 agent 를 보는 slot 각각에 cursor 부터 read → snapshot 수집 + advance.
        //     ★slot 목록을 먼저 clone★: cursors 와 log 를 같은 메서드 안에서 가변 접근해야 하는데,
        //     slots_for_agent 가 cursors 를 불변 빌림하면 그 사이 cursors.advance/clamp(가변)가 막힌다.
        //     slot 키만 미리 모아(소수 — 동시에 보는 창 수) borrow 충돌을 푼다.
        let slots: Vec<S> = self.cursors.slots_for_agent(agent_id).cloned().collect();
        let mut out: Vec<(S, Vec<u8>)> = Vec::new();
        for slot in slots {
            // ★deliverable 게이트(근원2)★: Channel 이 아직 등록 안 된 slot(미mount 창)은 read·snapshot·
            //   advance 를 통째로 건너뛴다 — 전달 못 할 구간을 advance 하면 영구 유실(이후 subscribe 가
            //   불가침 판정). cursor 는 멈춰 있다가 mount 시 subscribe_output(fresh 리셋+replay)이 채운다.
            //   ★멤버십 추적은 위 sync_viewers 가 이미 했다★ — 여기 스킵은 *진도(advance)* 만, slot 신설/
            //   제거는 deliverable 무관하게 sync_viewers 가 router 기준으로 한다(두 축 분리).
            if !deliverable.contains(&slot) {
                continue;
            }
            let cursor = match self.cursors.get(&slot) {
                Some(vc) => vc.cursor,
                None => continue, // 경합 방어(목록 수집 후 제거됨) — 없으면 스킵.
            };
            let (chunks, outcome) = log.read_from(cursor);
            // gap(Truncated) → clamp 후 재read. clamp 가 cursor 를 new_oldest 포함하도록 끌어올린다.
            if outcome == ReadOutcome::Truncated {
                // new_oldest = 현재 버퍼 oldest(read_from 이 Truncated 면 버퍼가 비어있지 않다).
                if let Some(new_oldest) = log.oldest_seq() {
                    self.cursors.clamp_cursors_for_agent(agent_id, new_oldest);
                    let new_cursor = self.cursors.get(&slot).and_then(|vc| vc.cursor);
                    let (chunks2, _o2) = log.read_from(new_cursor);
                    // ★불변식 박제(FIX-4)★: clamp 는 cursor 를 new_oldest 를 *포함*하도록(off-by-one 계약)
                    //   끌어올리므로, clamp 직후 재read 는 gap 0 = 반드시 Resumed 여야 한다(또 Truncated 면
                    //   clamp off-by-one 회귀 = 무한 재read·유실). 회귀 그물로 박는다(prod 은 영향 없음).
                    debug_assert_eq!(
                        _o2,
                        ReadOutcome::Resumed,
                        "Truncated clamp 후 재read 는 Resumed 여야(off-by-one 회귀)"
                    );
                    if let Some(last) = chunks2.last() {
                        let last_seq = last.seq;
                        for c in chunks2 {
                            out.push((slot.clone(), c.bytes.clone()));
                        }
                        self.cursors.advance(&slot, last_seq);
                    }
                }
                continue;
            }
            // Resumed/UpToDate — 보낼 게 있으면 snapshot + advance.
            if let Some(last) = chunks.last() {
                let last_seq = last.seq;
                for c in chunks {
                    out.push((slot.clone(), c.bytes.clone()));
                }
                self.cursors.advance(&slot, last_seq);
            }
        }
        out
    }

    /// ★현재 이 agent 를 보는 slot 집합을 store cursor 에 동기화(src-tauri 가 router 스냅샷에서 파생)★.
    ///
    /// 왜 필요한가(경계 — ADR-0012): "어느 창이 어느 agent 를 보나"는 layout/router(ViewManager 파생,
    /// Tauri 측) 권위다. core store 는 그 표를 모른다(순수). 그래서 src-tauri 가 `router.targets(agent)`
    /// 로 만든 **현재 viewer slot 집합**을 on_frame 직전에 이 메서드로 주입한다 — store 는 그 집합과
    /// 자기 cursor 를 맞춘다:
    /// - **신규 slot**(집합엔 있는데 cursor 없음) → `insert(slot, agent, None)`(fresh=처음부터 전체 replay).
    /// - **사라진 slot**(cursor 엔 있는데 집합에 없음) → `remove(slot)`(창이 그 agent 를 더는 안 봄).
    ///
    /// 반환 = 이번 동기화로 콘텐츠 버퍼를 drop 했는지(마지막 viewer 가 빠져 0개가 됨). 보통 `false`.
    /// ★이 메서드는 replay snapshot 을 반환하지 않는다★: 신규 slot 의 fresh replay 는 바로 뒤따르는
    /// `on_frame`(append 한 frame 까지 포함)이 cursor=None 부터 read 해 한 번에 내보낸다.
    ///
    /// ## ★세 진입점 cursor 역할 경계(deep 리뷰 급소 — TRD §3 정합)★
    /// 한 cursor 를 세 진입점이 만지지만 **역할이 겹치지 않게** 분리돼 있다(이중관리·race·중복 send 차단):
    /// - [`subscribe_agent`](Self::subscribe_agent) = **mount 1회 replay**(델타 0→1, layout). cursor 가
    ///   *없는* slot 만 fresh 신설 + 그 시점 버퍼 즉시 replay(조용한 agent 도 빈 화면 0 — 수용기준 5).
    /// - `sync_viewers`(이 메서드) = **on_frame 직전 router reconcile**(매 frame). 역할을 **"현재 router
    ///   기준 viewer 집합을 cursor 에 반영(신규 신설·사라진 slot 제거)"** 으로 좁힌다. ★신규 slot insert 는
    ///   `is_none()` 가드 뒤에서만★ — subscribe_agent 가 이미 None 신설 후 replay 로 advance 해 cursor 가
    ///   `Some(last)` 가 됐어도, 이 가드가 *재insert 를 건너뛴다* → **cursor 를 None 으로 되돌려 이미 보낸
    ///   구간을 중복 send 하는 것을 원천 차단**한다(race: layout subscribe_agent → 직후 첫 frame on_frame).
    ///   sync_viewers 는 cursor 값을 *되돌리지 않는다*(없을 때만 신설, 있으면 불가침) — 진도 전진은 on_frame
    ///   read 가, mount replay 는 subscribe_agent 가 단독 책임. 충돌 우선순위: **기존 cursor > 신규 None**.
    /// - [`drop_agent`](Self::drop_agent) = **배정 해제 폐기**(델타 1→0, layout). agent 의 모든 cursor 제거
    ///   + content drop. frame 도착과 **독립**이라 terminal(frame 0)이어도 정상 폐기(TRD §4).
    pub fn sync_viewers(&mut self, agent_id: AgentId, current_slots: &[S]) -> bool {
        use std::collections::HashSet;
        let want: HashSet<&S> = current_slots.iter().collect();
        // 사라진 slot 제거(이 agent 를 보던 cursor 중 want 에 없는 것).
        let have: Vec<S> = self.cursors.slots_for_agent(agent_id).cloned().collect();
        for slot in &have {
            if !want.contains(slot) {
                self.cursors.remove(slot);
            }
        }
        // 신규 slot 신설(want 에 있는데 cursor 없는 것) — fresh=None(처음부터 전체).
        for slot in current_slots {
            if self.cursors.get(slot).is_none() {
                self.cursors.insert(slot.clone(), agent_id, None);
            }
        }
        // 콘텐츠 신설(없으면 — frame 이 곧 채운다). 이미 있으면 재사용.
        self.content.entry(agent_id).or_default();
        // 마지막 viewer 가 빠졌으면 콘텐츠 폐기(생명주기 §4).
        if !self.cursors.agent_has_viewers(agent_id) {
            self.content.remove(&agent_id);
            self.epochs.remove(&agent_id);
            return true;
        }
        false
    }

    /// ★배정 트리거 운영 경로(layout 배정 델타 fresh=false → connection.rs `ReplaySlots` arm)★. 한 slot 이
    /// 에이전트를 보기 시작(mount/배정): **cursor 가 없을 때만** fresh 신설(None=처음부터 전체, PRD §3-1) +
    /// 그 시점 버퍼 [oldest~최신] replay snapshot 반환(§2 새 창 mount → 버퍼 replay).
    ///
    /// - 콘텐츠가 없으면 신설(빈 ring) — 데몬 `subscribe_from`(FromOldest) 가 채울 때까지 빈 snapshot.
    /// - 이미 다른 slot 이 보던 agent 면 콘텐츠 **재사용**(데몬 재요청 0). 그 slot 은 `read_from(None)`
    ///   으로 현재 버퍼 전체를 즉시 받아(끊긴 상태서도 replay — 수용기준 5) advance 한다.
    ///
    /// ## ★이미 cursor 가 있는 slot 은 불가침(정합 — deep 리뷰 급소)★
    /// cursor 가 *이미 있으면*(다른 진입점이 신설·진도 전진해 둠) **재신설하지 않고 빈 snapshot 을
    /// 반환**한다. None 으로 되돌려 replay 를 다시 주면 이미 본 구간을 중복 send 하기 때문이다(layout
    /// 델타와 on_frame 이 같은 slot 을 두고 race 하는 창). mount-즉시-replay 는 "처음 보는 slot" 에만
    /// 의미가 있고, 이어보던 slot 의 진도 전진은 [`on_frame`](Self::on_frame) read 가 단독 책임이다.
    ///
    /// ## ★`deliverable` 게이트(5차 FIX — mount replay 도 cursor advance ⟺ delivery)★
    /// 4차가 이 `subscribe` 를 배정 트리거 운영 경로에 배선했는데, 그땐 deliverable 무관하게 신설 즉시
    /// advance 했다 → **배정 시점에 그 창 webview 가 아직 mount 안 됨(Channel 미등록)이면**, 신설된 cursor 가
    /// 전달 못 할 구간까지 advance 돼 그 snapshot 은 flush 가 registry miss 로 스킵(유실)되는데 cursor 만
    /// 전진하는 [`on_frame`] 의 옛 결함이 mount 경로에 그대로 재현됐다. 그래서 [`on_frame`] 과 **동일 규율**을
    /// 적용한다 — slot 이 deliverable 에 **있을 때만** read/advance/emit, 없으면 **cursor 엔트리만 None 으로
    /// 신설(membership)** 하고 advance/snapshot 은 안 한다(값 None 유지). 미등록 slot 은 그 창이 mount 해
    /// `subscribe_output` 이 등록 트리거(`resubscribe_slot`, fresh 리셋+전체 replay)를 걸 때 무손실로 채워진다
    /// (등록 트리거는 호출부가 그 slot 을 항상 deliverable 에 넣어 즉시 replay — 빈 화면 0). 신설(membership)과
    /// 전진(advance)을 분리해 두 축(멤버십 ↔ 진도)이 섞이지 않게 한다(헤더 §`cursor advance ⟺ delivery`).
    pub fn subscribe(
        &mut self,
        slot: S,
        agent_id: AgentId,
        deliverable: &HashSet<S>,
    ) -> Vec<(S, Vec<u8>)> {
        // 콘텐츠 신설(없을 때만). 있으면 재사용.
        let log = self.content.entry(agent_id).or_default();
        // ★이미 cursor 가 있으면 불가침★: 재신설·replay 금지(중복 send 차단). 빈 snapshot.
        if self.cursors.get(&slot).is_some() {
            return Vec::new();
        }
        // ★cursor 신설(membership)은 deliverable 무관★: 라우팅에 배정됐으면 cursor 엔트리는 만든다(None=
        //   아직 안 읽음). 그래야 reconcile 이 "신설됐으나 미전달(값 None)" 을 복구 대상으로 식별할 수 있다.
        self.cursors.insert(slot.clone(), agent_id, None);
        // ★advance/emit 만 deliverable 게이트★: Channel 미등록 slot(미mount 창)은 전달 못 하므로 read·
        //   advance·snapshot 을 통째로 건너뛴다(값 None 유지). 전달 못 할 구간을 advance 하면 그 출력이
        //   영구 유실되기 때문(이후 reconcile/subscribe 가 전달된 것으로 오인). mount 시 등록 트리거가 채운다.
        if !deliverable.contains(&slot) {
            return Vec::new();
        }
        let (chunks, _outcome) = log.read_from(None);
        let mut out: Vec<(S, Vec<u8>)> = Vec::new();
        if let Some(last) = chunks.last() {
            let last_seq = last.seq;
            for c in chunks {
                out.push((slot.clone(), c.bytes.clone()));
            }
            self.cursors.advance(&slot, last_seq);
        }
        out
    }

    /// ★Channel 신규/재등록 시 cursor 강제 fresh 리셋 + 전체 replay(근원2 FIX)★. `subscribe` 와 달리
    /// **cursor 가 이미 있어도 None 으로 되돌린 뒤** 그 시점 버퍼 [oldest~최신] 을 전부 replay 한다.
    ///
    /// ## 왜 불가침(subscribe)이 아니라 강제 리셋인가
    /// `subscribe_output` invoke(webview mount = Channel registry insert)는 **그 창 viewer 의 (재)시작**
    /// 이다 — webview reload·팝업 재오픈처럼 Channel 이 교체되면, 옛 cursor 는 *옛 webview 가 본* 진도라
    /// 새 webview 엔 의미가 없다(새 xterm 은 빈 화면). 옛 cursor 를 그대로 이어보기(불가침)하면 새 창은
    /// 그 진도 이후만 받아 **이미 evict 됐거나 reload 전 구간을 영영 못 봐** 빈/잘린 화면이 된다. Channel
    /// 신규 등록 = viewer 재시작 = **전체 replay 가 맞다**(stale 이어보기 금지). 그래서 cursor 를 None 으로
    /// 리셋하고 버퍼 전체를 다시 준다. 같은 Channel 을 유지한 채의 정상 이어보기(on_frame read)와는 구분된다
    /// — 그건 advance 된 cursor 를 그대로 쓰고, 이 경로는 **Channel 등록 이벤트** 에서만 불린다.
    ///
    /// ★근원2 상보★: `on_frame` 의 deliverable 게이트가 "미mount 창의 cursor advance"를 막아 stale
    /// advance 를 방지하고, 이 메서드가 "mount 시 fresh 리셋 + 전체 replay"로 그 멈춘 cursor 를 무손실로
    /// 채운다 — 둘이 함께 "cursor advance ⟺ Channel delivery" 불변식의 양 끝을 막는다(advance 는 등록분만,
    /// 등록 시점은 전체 replay).
    ///
    /// ## ★`deliverable` 게이트(5차 FIX — 일관성)★
    /// 운영의 등록 트리거(`subscribe_output` invoke)는 registry insert *직후* 호출되므로 그 slot 은 항상
    /// deliverable 안이라 실제로 게이트에 걸리지 않는다. 그래도 `on_frame`·`subscribe` 와 **같은 규율**을
    /// 명시적으로 들어, 어떤 경로로 미등록 slot 에 불리더라도 stale advance 가 새지 않게 한다 — slot 이
    /// deliverable 밖이면 cursor 만 None 으로 리셋(membership)하고 advance/snapshot 은 안 한다(값 None 유지).
    /// reconcile 이 "값 None + deliverable" 을 복구 대상으로 잡으므로, 이 게이트가 그 식별을 신뢰 가능하게 한다.
    pub fn resubscribe_slot(
        &mut self,
        slot: S,
        agent_id: AgentId,
        deliverable: &HashSet<S>,
    ) -> Vec<(S, Vec<u8>)> {
        let log = self.content.entry(agent_id).or_default();
        // ★강제 fresh★: 있든 없든 None 으로(재insert) — 옛 webview 진도를 버리고 전체 replay.
        self.cursors.insert(slot.clone(), agent_id, None);
        // ★advance/emit 만 deliverable 게이트★: 미등록 slot 은 전달 못 하므로 read·advance·snapshot 스킵
        //   (값 None 유지 — on_frame/subscribe 와 동형). 그 slot 은 다음 등록/reconcile 이 채운다.
        if !deliverable.contains(&slot) {
            return Vec::new();
        }
        let (chunks, _outcome) = log.read_from(None);
        let mut out: Vec<(S, Vec<u8>)> = Vec::new();
        if let Some(last) = chunks.last() {
            let last_seq = last.seq;
            for c in chunks {
                out.push((slot.clone(), c.bytes.clone()));
            }
            self.cursors.advance(&slot, last_seq);
        }
        out
    }

    /// ★갭① mount-즉시-replay(agent 단위 진입점 — layout 구독 델타 0→1)★. 한 agent 가 *처음으로* 어느
    /// 창엔가 보이기 시작하면(`SubscriptionDelta::to_subscribe`), 그 agent 를 보는 **현재 viewer slot
    /// 집합**(src-tauri 가 `router.targets(agent)` 로 파생)에 대해 cursor 없는 slot 만 fresh 신설하고 그
    /// 시점 버퍼 전체를 즉시 replay 한다. 반환 snapshot 을 **버퍼 락 밖에서 Channel send** 한다(TRD §1).
    ///
    /// ## ★운영 미사용 — 폐기 후보(다음 세션 정본 오인 차단)★
    /// 운영은 agent 단위가 아니라 **slot 단위**(`SubscriptionDelta::slots_to_replay` → connection.rs
    /// `ReplaySlots` arm → [`resubscribe_slot`])로 mount replay 한다 — 1→2(이미 보던 agent 에 새 창)를
    /// slot 쌍 diff 로 잡아야 하고(agent-union diff 만으론 못 잡음, FIX-3), Channel 등록=viewer 재시작이라
    /// 불가침이 아니라 fresh 리셋이 맞기 때문(근원2). 이 agent 단위 진입점은 **단위테스트에서만** 쓰인다.
    /// 신규 배선은 slot 단위 `resubscribe_slot` 을 쓴다(이걸 정본으로 오인해 부르면 1→2 누락·reload 빈 화면).
    ///
    /// ## 왜 agent 단위인가 + 왜 mount 즉시인가(TRD §2·수용기준 5)
    /// 구독 델타는 agent 단위(0→1 토글)다 — "그 agent 를 보는 모든 slot" 을 한 번에 깨운다. **on_frame 의
    /// `sync_viewers` 경로만으론 다음 frame 이 와야 replay 가 나가** 조용한 agent·재연결 대기 중 새 창이
    /// 다음 출력까지 빈 화면이다(수용기준 5 위반). 이 진입점이 그 갭을 메운다 — frame 도착과 무관하게
    /// mount 시점에 즉시 버퍼를 flush.
    ///
    /// ## ★정합: cursor 없는 slot 만(중복 send 차단)★
    /// 각 slot 은 [`subscribe`](Self::subscribe) 를 거쳐 **cursor 가 없을 때만** 신설·replay 한다 — 이미
    /// 보던 slot(on_frame/sync_viewers 가 진도 전진해 둠)은 불가침이라 cursor 후퇴·중복 replay 0.
    /// 콘텐츠가 빈 신규 agent 면 빈 snapshot(데몬 `subscribe_from` FromOldest 가 채운 뒤 on_frame 이 전달).
    pub fn subscribe_agent(
        &mut self,
        agent_id: AgentId,
        current_slots: &[S],
        deliverable: &HashSet<S>,
    ) -> Vec<(S, Vec<u8>)> {
        // 콘텐츠 신설(없을 때만 — 데몬 replay 가 채운다). 이미 있으면 재사용(데몬 재요청 0).
        self.content.entry(agent_id).or_default();
        let mut out: Vec<(S, Vec<u8>)> = Vec::new();
        for slot in current_slots {
            // subscribe 가 "cursor 없을 때만 신설 + (deliverable 이면) replay" 가드를 들어 정합을 단일
            //   지점에 모은다. deliverable 게이트도 subscribe 가 일관 적용(미등록 slot=membership 만).
            out.extend(self.subscribe(slot.clone(), agent_id, deliverable));
        }
        out
    }

    /// 슬롯 배정 해제(창 닫힘/재배정): cursor 제거 → 그 agent 를 보는 slot 이 0개가 되면 콘텐츠 폐기
    /// (TRD §4 생명주기 — 캐시는 어느 창엔가 배정된 동안만, 재배정 시 데몬 replay 로 새 버퍼).
    ///
    /// 반환 = 콘텐츠 버퍼를 실제로 drop 했는지(`true`=마지막 viewer 빠져 폐기 / `false`=아직 viewer
    /// 남았거나 없던 slot). 호출자(메모리 회계·로깅)가 쓸 수 있다.
    pub fn unsubscribe(&mut self, slot: &S) -> bool {
        let Some(agent_id) = self.cursors.remove(slot) else {
            return false; // 없던 slot.
        };
        if !self.cursors.agent_has_viewers(agent_id) {
            // 마지막 viewer 가 빠짐 → 콘텐츠 + epoch 태그 폐기(누수 0 — terminal agent 도 자동 해소).
            self.content.remove(&agent_id);
            self.epochs.remove(&agent_id);
            return true;
        }
        false
    }

    /// ★갭② 생명주기 폐기(agent 단위 진입점 — layout 구독 델타 1→0)★. 한 agent 가 *더는 어느 창에도*
    /// 안 보이게 되면(`SubscriptionDelta::to_unsubscribe`), 그 agent 의 **모든 cursor 제거 + content/epoch
    /// drop**. 반환 = 실제로 콘텐츠를 들고 있다가 폐기했으면 `true`.
    ///
    /// ## ★frame 도착과 독립(TRD §4 — 폐기 트리거 = View 배정 해제)★
    /// 폐기를 `sync_viewers`(frame 도착 시점)에만 묶으면, **terminal(Killed/Exited)이라 더는 frame 이 안
    /// 오는데 창은 열려 있던** agent 가 배정 해제돼도 cursor/버퍼가 영영 안 빠진다(frame 트리거 부재).
    /// 이 진입점은 layout 델타(배정 해제)에서 직접 불려 frame 과 **독립**으로 폐기한다 — terminal+창 닫힘
    /// 케이스에서 정상 폐기(누수 0). cursor 가 없던(이미 폐기/미배정) agent 면 no-op `false`.
    ///
    /// ## ★운영 미사용 — 폐기 후보(다음 세션 정본 오인 차단)★
    /// 운영은 agent 단위가 아니라 **slot 단위**(`SubscriptionDelta::slots_to_drop` → connection.rs
    /// `DropSlots` arm → [`unsubscribe`](Self::unsubscribe))로 폐기한다 — 2→1(여러 창 중 하나만 닫힘)을
    /// slot 쌍 diff 로 잡아야 죽은 cursor 가 안 남기 때문(FIX-3). full 누수 reconcile 은 [`sweep_orphans`]
    /// 가 별도로 흡수. 이 agent 단위 진입점은 **단위테스트에서만** 쓰인다(신규 배선은 slot 단위를 쓴다).
    pub fn drop_agent(&mut self, agent_id: AgentId) -> bool {
        // 그 agent 를 보는 모든 slot cursor 제거(slots_for_agent 로 모아 한꺼번에 — borrow 충돌 회피).
        let slots: Vec<S> = self.cursors.slots_for_agent(agent_id).cloned().collect();
        for slot in &slots {
            self.cursors.remove(slot);
        }
        // 콘텐츠/epoch 태그 폐기. content 가 있었으면 true(실제 메모리 회수), 없었으면 false(no-op).
        let had_content = self.content.remove(&agent_id).is_some();
        self.epochs.remove(&agent_id);
        had_content
    }

    /// 재연결 시 데몬 재구독 `after_seq`(축 A — 클라에 없는 것만 받기). = 버퍼 최신 seq.
    /// 버퍼 없음/빈 버퍼면 `None`(데몬이 FromOldest 로 전체 보냄). **창 cursor(축 B)와 무관** — 이게
    /// 두 축 분리의 코드 경계다(이 함수는 cursor 를 절대 안 본다).
    ///
    /// ★미렌더 무손실 = 클라 버퍼 잔량 한정(의도된 trade-off — ADR-0040/TRD §3)★: after_seq 가 버퍼
    /// 최신이라, 미렌더 창의 무손실 복구는 **클라 버퍼에 아직 남은 구간**에 한정된다. 버퍼 최신 < 데몬
    /// oldest 까지 끊김이 길어져 그 사이가 데몬에서도 evict 되면 그 구간은 **불가피 유실**이고 잘림 미표시
    /// 다(TRD §3 gap 항목). 이건 결함이 아니라 "데몬=장기 원본 / 클라=뷰 채우기 캐시" 분담에서 온 명시적
    /// 설계 결정 — `min_render_seq`(가장 뒤처진 창 합산) 모델로 되돌리면 ADR-0040 위반이므로 회귀 금지.
    pub fn resubscribe_after_seq(&self, agent_id: AgentId) -> Option<u64> {
        self.content.get(&agent_id).and_then(|log| log.latest_seq())
    }

    /// ★고아 cursor/콘텐츠 sweep(FIX-1 — drop_slots full 누수 reconcile)★. `keep` = 현재 router 에
    /// 실재하는(= 어느 창엔가 보이는) slot 집합. 그에 **없는 cursor 를 전부 제거**하고, 그 결과 viewer 가
    /// 0 이 된 agent 의 콘텐츠/epoch 태그도 폐기한다. 제거한 cursor 수를 반환(0=정합 상태, 로깅용).
    ///
    /// ## 왜 필요한가(누수 경로 — `try_enqueue` silent drop)
    /// `drop_slots` 는 bounded mpsc `try_send` 라 채널 full 이면 **조용히 drop** 된다(자가복구 트리거 부재 —
    /// 안 보이는 agent 는 frame 이 안 와 `sync_viewers` 가 정리할 기회도 없다) → 그 (window,agent) cursor 가
    /// **영구 잔존**한다(마지막 cursor 였다면 콘텐츠 버퍼까지 leak). 이 sweep 을 connect/재연결 진입
    /// resubscribe 직후 1회 돌려, layout 권위(router)와 store cursor 를 강제 정합화한다 — full 로 새어나간
    /// drop 을 흡수하는 reconcile 경로다. router 가 SSOT(ADR-0035)라 keep 에 없으면 더는 viewer 아님이 확실.
    ///
    /// ★무손실 충돌 없음★: keep 에 *있는* slot 은 절대 안 건드린다(cursor 보존 → 그 창 진도 유지). 제거
    /// 대상은 layout 이 이미 떼어낸(router 에 없는) slot 뿐이라, 지우는 게 정답이다(생명주기 TRD §4).
    ///
    /// ## ★keep 최신성 전제(load-bearing — 단일 actor 직렬이라 충돌 없음)★
    /// 이 sweep 이 무충돌인 건 **keep(`router.current_slots()`)이 호출 시점의 최신 layout 스냅샷일 때만**이다.
    /// keep 이 낡으면(그 사이 layout 이 또 바뀜) 방금 다시 배정된 slot 을 고아로 오인해 지울 수 있다. 이게
    /// 안전한 이유: 호출자(connection.rs `resubscribe_and_sweep`)가 이 함수와 그 위 양방향 reconcile 을
    /// **연결 actor 단일 스레드**의 한 흐름(buffer 락 1회 보유) 안에서 돌리고, layout rebuild→delta enqueue 는
    /// 다른 경로지만 store 조작은 전부 이 actor 의 select! arm 으로 직렬화된다(ADR-0006 — actor 가 ViewManager
    /// 락을 잡지 않아 데드락도 없음). 즉 reconcile 함수 *내부*에서 keep 읽기와 sweep/replay 가 원자적이다
    /// (그 사이 다른 store 조작이 끼어들지 못함). 더 새 layout 변화는 다음 frame 의 sync_viewers·다음 Resync·
    /// 재연결 reconcile 이 따라잡는다(eventual 정합).
    pub fn sweep_orphans(&mut self, keep: &HashSet<S>) -> usize {
        // 현재 cursor 를 든 모든 slot 중 keep 에 없는 것을 모은다(borrow 충돌 회피 — 먼저 수집).
        let orphans: Vec<S> = self
            .cursors
            .all_slots()
            .filter(|s| !keep.contains(s))
            .cloned()
            .collect();
        for slot in &orphans {
            // unsubscribe 가 cursor 제거 + 마지막이면 콘텐츠/epoch drop 까지 한 번에(생명주기 단일 지점).
            self.unsubscribe(slot);
        }
        orphans.len()
    }

    /// ★양방향 slot reconcile(FIX-1 — 유실된 replay 복구 + 고아 sweep)★. `keep` = 현재 router 에 실재하는
    /// `(slot, agent)` 쌍 전체(layout 권위 SSOT, ADR-0035). connect/재연결/Resync 진입 시 store cursor 를
    /// 이 집합과 **양방향**으로 맞춘다 — `sweep_orphans`(한 방향: 고아 제거)를 멱등 reconcile 로 일반화한다:
    /// - **(a) 유실 replay 복구**: keep 에 *있는데* cursor 가 없는 slot(router 엔 배정됐으나 cursor 미신설) →
    ///   `resubscribe_slot` 으로 fresh 신설 + 그 시점 버퍼 전체 replay snapshot 수집. `replay_slots` 가 채널
    ///   full 로 silent drop 돼 신설 못 한 slot 을 여기서 복구한다(새 창 빈 화면 차단).
    /// - **(b) 고아 sweep**: keep 에 *없는* cursor → 제거(`drop_slots` full 누수 흡수 — 기존 sweep_orphans 동작).
    ///
    /// ## ★멱등 + 복구 판정 = "실제 전달 진도"(값) 기준(5차 FIX — load-bearing)★
    /// 4차의 복구 가드는 **cursor 엔트리 존재 여부**(`is_some()`)로 "전달 완료"를 판정해 엔트리 있는 slot 을
    /// 무조건 불가침으로 뒀다. 그 전제가 거짓이다 — `subscribe`(배정 트리거)가 미mount 창에 **cursor 엔트리는
    /// 신설하되 값은 None(미전달)** 으로 남길 수 있기 때문이다(5차 deliverable 게이트). 그 slot 이 등록 전에
    /// 배정만 됐다가 등록 트리거(`replay_slots fresh=true`)마저 채널 full 로 silent drop 되면, 옛 가드는
    /// "엔트리 있음=완료"로 보고 복구를 건너뛰어 **그 창 영구 빈 화면**이 났다(Codex BLOCK). 그래서 판정을
    /// 엔트리 유무가 아니라 **cursor 값(실제 전달 진도)** 으로 바로잡는다:
    /// - **엔트리 없음**(유실된 신설 — replay_slots full drop) → `resubscribe_slot` 로 fresh 신설 + replay.
    /// - **값 `None` + deliverable** → 복구 대상. `resubscribe_slot` 로 전체 replay.
    /// - **값 `None` + 미deliverable**(아직 미mount) → 건드리지 않음(membership 유지). 그 창 mount 시 등록
    ///   트리거가 채운다. deliverable 무관하게 replay 하면 전달 못 할 구간을 advance 해 다시 영구 유실.
    /// - **값 `Some(_)`(전달 진행 중)** → 불가침(`on_frame` read 단독 책임). 여러 번 돌려도 중복 replay 0.
    ///
    /// ## ★값 `None` 의 발생 경로는 *둘* — "한 번도 전달 안 됨" 으로 단정 금지(load-bearing)★
    /// cursor 값 `None` 은 "신설됐으나 미전달" 한 가지만 의미하지 않는다. None 이 생기는 경로는 **둘**이다:
    /// 1. **미deliverable 신설**(`subscribe` 가 배정만 됐고 Channel 미등록인 slot 에 cursor 엔트리만 None
    ///    으로 남김) — 진짜 한 번도 전달 안 된 경우.
    /// 2. **epoch 전환 reset**(`on_frame` 이 `frame.epoch ≠ 태그` 를 보고 그 agent 의 모든 cursor 를
    ///    `reset_cursors_for_agent(agent, None)` 로 Some→None 으로 되돌림, ADR-0007) — 새 스트림이라 *이미
    ///    전달했던* cursor 도 None 으로 강등된다(전달 이력이 있어도 값은 None).
    ///
    /// ★그래도 None + deliverable → 전체 replay 가 옳다(두 경로 공통)★: epoch 전환은 cursor 만이 아니라
    /// **콘텐츠 ring 도 함께 reset**(`on_frame` 의 `log.reset()`)한다 — 옛 epoch 버퍼는 비워지고 새 epoch
    /// frame 만 다시 쌓인다. 그래서 None + deliverable 로 `read_from(None)` 하면 *옛 구간 재전송이 아니라*
    /// **새 epoch 의 첫 전달**(reset 된 ring 의 현재 내용)이라 중복이 아니다(경로 1 도 "처음부터" 가 맞다).
    /// 설령 좁은 race(epoch 전환과 reconcile 사이)로 같은 bytes 가 한 번 더 나가도 프론트 seq dedup
    /// (ADR-0037 2차망)이 흡수한다 — 정확성은 깨지지 않는다.
    ///
    /// `deliverable` = 현재 Channel 이 등록된 slot 집합(호출자가 buffer 락 *전*에 `registered_labels` 로 떠
    /// 주입 — `on_frame` 호출부와 동일 패턴, ADR-0006). `resubscribe_slot` 자체도 게이트를 들지만, 미전달
    /// (값 None) slot 의 복구 *여부* 를 여기서 deliverable 로 거르므로 미등록 slot 엔 헛 replay 시도조차 안 한다.
    ///
    /// 반환 = (a) 복구 replay snapshot(`Vec<(S, bytes)>` — 락 밖 Channel send) + (b) 제거한 고아 cursor 수.
    /// 콘텐츠 신설은 안 한다(빈 신규 agent 는 데몬 wire 재구독이 곧 채움 — replay 는 *이미 있는* 버퍼만).
    pub fn reconcile_slots(
        &mut self,
        keep: &[(S, AgentId)],
        deliverable: &HashSet<S>,
    ) -> (Vec<(S, Vec<u8>)>, usize) {
        // keep 의 slot 키만 모은 집합(고아 판정용 — borrow 충돌 회피 위해 먼저 만든다).
        let keep_set: HashSet<S> = keep.iter().map(|(s, _)| s.clone()).collect();
        // (b) 고아 sweep — keep 에 없는 cursor 제거(마지막이면 콘텐츠/epoch drop).
        let removed = self.sweep_orphans(&keep_set);
        // (a) 유실 replay 복구 — "실제 전달 진도(cursor 값)" 기준으로 복구 대상을 가린다(엔트리 유무 아님).
        let mut out: Vec<(S, Vec<u8>)> = Vec::new();
        for (slot, agent_id) in keep {
            match self.cursors.get(slot).map(|vc| vc.cursor) {
                // 값 Some(_) = 정상 전달 중 → 불가침(on_frame 단독). 중복 replay 0.
                Some(Some(_)) => continue,
                // 값 None = 미전달 신설 *또는* epoch 전환 reset(ADR-0007 — ring 도 reset됨, 위 헤더 참조).
                //   둘 다 deliverable 이면 복구(전체 replay=새 epoch 첫 전달), 아니면 membership 유지(미mount).
                Some(None) => {
                    if !deliverable.contains(slot) {
                        continue;
                    }
                }
                // 엔트리 없음 = 유실된 신설(replay_slots full drop). deliverable 이면 신설+replay, 아니면
                //   resubscribe_slot 이 게이트로 cursor 만 None 신설(membership) — 둘 다 resubscribe_slot 위임.
                None => {}
            }
            // 복구: resubscribe_slot 이 fresh 신설(또는 None 리셋) + deliverable 이면 전체 replay.
            out.extend(self.resubscribe_slot(slot.clone(), *agent_id, deliverable));
        }
        (out, removed)
    }

    /// 현재 콘텐츠 버퍼를 든 agent 목록(재구독 순회용 — 호출자가 각 agent 에 resubscribe_after_seq 로
    /// after_seq 를 채워 wire Subscribe 재전송). 결정론(테스트 재현)을 위해 정렬해 반환.
    pub fn buffered_agents(&self) -> Vec<AgentId> {
        let mut v: Vec<AgentId> = self.content.keys().copied().collect();
        v.sort();
        v
    }

    /// 이 agent 를 보는 slot 이 1개라도 있나(테스트·진단용 — 생명주기 단언).
    pub fn agent_has_viewers(&self, agent_id: AgentId) -> bool {
        self.cursors.agent_has_viewers(agent_id)
    }

    /// 콘텐츠 버퍼를 든 고유 agent 수(메모리 회계·테스트용).
    pub fn buffered_agent_count(&self) -> usize {
        self.content.len()
    }

    /// ★테스트 전용 — "그 agent 의 현재 모든 cursor slot 이 deliverable"인 on_frame★. 근원2 deliverable
    /// 게이트 도입 전의 기존 테스트(Channel 등록 race 미고려, 모든 slot 이 전달 가능 가정)를 그대로
    /// 유지하기 위한 편의 래퍼다 — 그 agent 를 보는 slot 전부를 deliverable 집합으로 넣어 on_frame 한다.
    /// 근원2 *전용* 회귀 테스트는 이 래퍼가 아니라 `on_frame(.., &deliverable)` 을 직접 불러 게이트를 검증한다.
    #[cfg(test)]
    fn on_frame_all(
        &mut self,
        agent_id: AgentId,
        frame_epoch: u64,
        seq: u64,
        bytes: Vec<u8>,
    ) -> Vec<(S, Vec<u8>)> {
        let deliverable: HashSet<S> = self.cursors.slots_for_agent(agent_id).cloned().collect();
        self.on_frame(agent_id, frame_epoch, seq, bytes, &deliverable)
    }

    /// ★테스트 전용 — slot 의 현재 cursor 값 조회(근원2 advance 단언)★. `Some(None)` = slot 존재·아직
    /// 안 읽음, `Some(Some(s))` = seq s 까지 읽음, `None` = slot 없음.
    #[cfg(test)]
    fn cursor_for_test(&self, slot: &S) -> Option<Option<u64>> {
        self.cursors.get(slot).map(|vc| vc.cursor)
    }

    /// ★테스트 전용 — "이 slot 은 deliverable"인 subscribe★. deliverable 게이트 도입 전 기존 테스트
    /// (모든 slot 이 전달 가능 가정)를 유지하기 위한 편의 래퍼. 게이트 *전용* 테스트는 직접 `subscribe(.., &set)` 호출.
    #[cfg(test)]
    fn subscribe_all(&mut self, slot: S, agent_id: AgentId) -> Vec<(S, Vec<u8>)> {
        let deliverable: HashSet<S> = std::iter::once(slot.clone()).collect();
        self.subscribe(slot, agent_id, &deliverable)
    }

    /// ★테스트 전용 — "이 slot 은 deliverable"인 resubscribe_slot★.
    #[cfg(test)]
    fn resubscribe_slot_all(&mut self, slot: S, agent_id: AgentId) -> Vec<(S, Vec<u8>)> {
        let deliverable: HashSet<S> = std::iter::once(slot.clone()).collect();
        self.resubscribe_slot(slot, agent_id, &deliverable)
    }

    /// ★테스트 전용 — "current_slots 전부 deliverable"인 subscribe_agent★.
    #[cfg(test)]
    fn subscribe_agent_all(&mut self, agent_id: AgentId, current_slots: &[S]) -> Vec<(S, Vec<u8>)> {
        let deliverable: HashSet<S> = current_slots.iter().cloned().collect();
        self.subscribe_agent(agent_id, current_slots, &deliverable)
    }

    /// ★테스트 전용 — "keep 의 slot 전부 deliverable"인 reconcile_slots★. 게이트 *전용* 테스트는 직접
    /// `reconcile_slots(.., &set)` 으로 부분 deliverable 을 검증한다.
    #[cfg(test)]
    fn reconcile_slots_all(&mut self, keep: &[(S, AgentId)]) -> (Vec<(S, Vec<u8>)>, usize) {
        let deliverable: HashSet<S> = keep.iter().map(|(s, _)| s.clone()).collect();
        self.reconcile_slots(keep, &deliverable)
    }
}

impl<S> Default for OutputViewStore<S>
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

    /// (slot, bytes) snapshot 에서 한 slot 의 bytes 들을 이어붙인다(검증 편의 — frame=1byte 가정).
    fn collected(out: &[(u32, Vec<u8>)], slot: u32) -> Vec<u8> {
        out.iter()
            .filter(|(s, _)| *s == slot)
            .flat_map(|(_, b)| b.iter().copied())
            .collect()
    }

    // ── 멀티뷰: 같은 agent N slot 독립 cursor, 공유 콘텐츠 1벌 ──────────────────────────────

    #[test]
    fn multiview_independent_cursors_share_one_content() {
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        // slot 1,2 가 같은 agent A 를 본다(콘텐츠 1벌 + cursor 2개).
        store.subscribe_all(1, a);
        store.subscribe_all(2, a);
        assert_eq!(store.buffered_agent_count(), 1, "콘텐츠는 agent 당 1벌");

        // frame 도착 → 두 slot 에 동일 bytes fan-out(각자 cursor 독립 전진).
        let out = store.on_frame_all(a, 0, 0, b"x".to_vec());
        assert_eq!(collected(&out, 1), b"x");
        assert_eq!(collected(&out, 2), b"x");

        // 다음 frame 도 둘 다.
        let out2 = store.on_frame_all(a, 0, 1, b"y".to_vec());
        assert_eq!(collected(&out2, 1), b"y");
        assert_eq!(collected(&out2, 2), b"y");
    }

    #[test]
    fn frame_for_unviewed_agent_buffers_but_sends_nothing() {
        // 아무 slot 도 안 보는 agent 의 frame: 콘텐츠엔 append(축 A 재구독 대비) 되나 보낼 slot 0.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        let out = store.on_frame_all(a, 0, 0, b"x".to_vec());
        assert!(out.is_empty(), "보는 slot 없으면 fan-out 0");
        assert_eq!(store.buffered_agent_count(), 1, "그래도 콘텐츠엔 버퍼링");
        assert_eq!(store.resubscribe_after_seq(a), Some(0), "축 A: 버퍼 최신=0");
    }

    // ── 새 창 늦게 mount → replay(§2) ───────────────────────────────────────────────────

    #[test]
    fn late_mount_replays_existing_buffer() {
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        // slot1 이 보는 동안 3건 도착.
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());
        store.on_frame_all(a, 0, 2, b"c".to_vec());

        // slot2 가 늦게 mount → 그 시점 버퍼 전체(abc) 를 즉시 replay 받는다.
        let replay = store.subscribe_all(2, a);
        assert_eq!(
            collected(&replay, 2),
            b"abc",
            "늦게 mount 한 창도 전체 replay"
        );

        // 이후 새 frame 은 둘 다(slot1 은 이미 abc 봤으니 d 만, slot2 도 d 만).
        let out = store.on_frame_all(a, 0, 3, b"d".to_vec());
        assert_eq!(collected(&out, 1), b"d");
        assert_eq!(collected(&out, 2), b"d");
    }

    #[test]
    fn mount_on_empty_buffer_replays_nothing() {
        // 콘텐츠가 아직 빈 신규 agent 에 mount → replay 0(데몬 subscribe_from 이 채울 때까지).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        let replay = store.subscribe_all(1, a);
        assert!(replay.is_empty());
        assert_eq!(store.buffered_agent_count(), 1, "빈 콘텐츠 신설됨");
    }

    // ── 재연결 두 축 분리(§3) ────────────────────────────────────────────────────────────

    #[test]
    fn resubscribe_after_seq_is_buffer_latest_not_cursor() {
        // ★두 축 분리 핵심★: 재구독 after_seq(축 A)=버퍼 최신, 창 cursor(축 B)와 무관.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a); // slot1 따라감
        store.subscribe_all(2, a); // slot2 미렌더로 둘 것
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());
        store.on_frame_all(a, 0, 2, b"c".to_vec());
        // 축 A: 버퍼 최신 = 2(창 진도와 무관 — 두 창 다 c 까지 봤든 말든 버퍼 최신만 본다).
        assert_eq!(store.resubscribe_after_seq(a), Some(2));
    }

    #[test]
    fn unrendered_window_lossless_after_reconnect() {
        // ★축 B 무손실★: 미렌더 창(cursor 보존)이 재연결 후에도 자기 cursor 부터 무손실 read.
        // 시나리오: slot1 은 mount 직후 frame 을 받지만, slot2 는 mount 만 하고(빈 버퍼) 끊김 →
        //           재연결 후 데몬이 after_seq(축A) 이후를 append → slot2 가 처음부터 전체 받음.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.subscribe_all(2, a); // slot2: cursor=None(아직 아무것도 안 읽음)

        // (끊김 전) frame 0,1 도착 → 둘 다 받음.
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());

        // slot3 가 "재연결 직후 새로 mount"(미렌더 창 모사) — cursor=None 이라 버퍼 전체(ab) 받음.
        let r3 = store.subscribe_all(3, a);
        assert_eq!(
            collected(&r3, 3),
            b"ab",
            "재연결 중 새 창도 끊긴 버퍼 즉시 replay"
        );

        // 재연결 후 데몬이 after_seq=1 이후(seq 2)만 append → 세 창 모두 c 받음(slot3 포함, 무손실).
        let out = store.on_frame_all(a, 0, 2, b"c".to_vec());
        assert_eq!(collected(&out, 1), b"c");
        assert_eq!(collected(&out, 2), b"c");
        assert_eq!(collected(&out, 3), b"c");
    }

    // ── gap(Truncated) clamp 후 new_oldest 포함(§3 gap, FIX-2 계약) ──────────────────────

    #[test]
    fn gap_truncated_clamps_and_includes_new_oldest() {
        // 버퍼 evict 로 oldest 가 올라가 뒤처진 창이 gap 을 만나면, clamp 후 재read 가 new_oldest 부터
        // 무손실로 나와야 한다(off-by-one 계약 — new_oldest 자체를 건너뛰지 않음).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        // slot1 이 seq 0 까지만 따라잡게 한다(첫 frame 만 받고 멈춤 가정 — advance 는 on_frame 이 함).
        store.on_frame_all(a, 0, 0, b"x".to_vec()); // slot1 cursor → Some(0)

        // slot2 를 mount 하되, 곧바로 대량 frame 으로 버퍼를 evict 시켜 slot2 cursor 가 oldest 밑이 되게.
        // (slot2 는 cursor=None 으로 시작하지만, 아래 on_frame 들이 slot1·slot2 둘 다 전진시킨다.
        //  대신 "뒤처진 창"을 직접 만들기 위해 slot2 cursor 를 옛 위치에 고정하는 대신, 1단계
        //  read_from 의 Truncated 분기를 store 가 올바로 타는지를 본다 — 대량 evict 후 늦은 mount.)
        for seq in 1..6000u64 {
            store.on_frame_all(a, 0, seq, vec![b'y']);
        }
        // 이제 버퍼 oldest 가 0 보다 한참 위(evict). 늦게 mount 하는 slot3 = cursor None → Resumed(전체).
        let oldest_after = store.resubscribe_after_seq(a); // = latest
        assert_eq!(oldest_after, Some(5999));

        // slot3 늦게 mount → 버퍼에 남은 전체(oldest~5999)를 무손실로 받는다(gap 아님 — None=전체).
        let r3 = store.subscribe_all(3, a);
        // 남은 개수 = 4096(event cap). 첫 바이트가 evict 안 된 oldest 의 것.
        assert_eq!(r3.len(), 4096, "버퍼에 남은 전체를 replay");
    }

    #[test]
    fn gap_truncated_path_recovers_after_drop_and_late_remount() {
        // on_frame 의 Truncated 분기를 store API 로 자연 유발하는 경로: 한 slot 이 낮은 seq 까지 본 뒤
        // unsubscribe(콘텐츠 drop) → 같은 seq 공간에서 데몬이 *evict 된 뒤쪽만* 다시 채우는(=Truncated)
        // 상황은 store 레벨에선 drop 으로 콘텐츠가 비워져 발생하지 않는다. 대신 1단계 read_from 의
        // Truncated 경계를 store 가 올바로 위임하는지는 buffer 단위테스트가 커버하므로, 여기서는
        // "대량 evict 후 새 mount 가 버퍼에 남은 전체를 무손실로 받는다"(gap 없는 정상 replay)를 본다.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        for seq in 0..5000u64 {
            store.on_frame_all(a, 0, seq, vec![b'z']);
        }
        // 버퍼는 4096 칸으로 evict 됨. 늦은 mount(slot2, cursor=None)는 남은 전체를 무손실로 받는다.
        let r2 = store.subscribe_all(2, a);
        assert_eq!(
            r2.len(),
            4096,
            "evict 후 남은 전체(4096)를 새 창이 무손실 replay"
        );
        // 마지막 frame(seq 4999)이 포함됐는지(tail 무손실).
        let out = store.on_frame_all(a, 0, 5000, vec![b'!']);
        assert_eq!(collected(&out, 1), b"!");
        assert_eq!(collected(&out, 2), b"!");
    }

    // ── epoch 전환: 해당 agent 만 reset, 다른 agent 보존(§4b) ────────────────────────────

    #[test]
    fn epoch_switch_resets_only_target_agent() {
        let a = aid(1);
        let b = aid(2);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.subscribe_all(2, b);
        // a: epoch 0 에서 2건, b: epoch 0 에서 1건.
        store.on_frame_all(a, 0, 0, b"a0".to_vec());
        store.on_frame_all(a, 0, 1, b"a1".to_vec());
        store.on_frame_all(b, 0, 0, b"b0".to_vec());

        // a 가 epoch 1 로 전환(새 스트림 seq 0) → a 콘텐츠/커서만 reset, b 보존.
        let out = store.on_frame_all(a, 1, 0, b"A0".to_vec());
        // a slot1 은 새 스트림 첫 출력(A0)을 받는다(옛 a0/a1 은 reset 으로 버려짐).
        assert_eq!(collected(&out, 1), b"A0");
        // a 버퍼 최신 = 새 스트림의 0(reset 후 append).
        assert_eq!(
            store.resubscribe_after_seq(a),
            Some(0),
            "a epoch 전환 후 새 seq 0"
        );
        // b 는 그대로(epoch 0, seq 0).
        assert_eq!(store.resubscribe_after_seq(b), Some(0));

        // b 에 epoch 0 frame 더 와도 정상 이어짐(a 의 전환에 영향 0).
        let outb = store.on_frame_all(b, 0, 1, b"b1".to_vec());
        assert_eq!(collected(&outb, 2), b"b1");
    }

    #[test]
    fn epoch_switch_old_high_seq_then_new_low_seq_not_dropped() {
        // ★회귀 가드★: 옛 epoch 에서 높은 seq 까지 갔다가 새 epoch 의 낮은 seq(0)로 전환 시 drop 0.
        // (min 모델의 옛 결함 — 높은 render_seq 가 새 낮은 seq 를 drop — 이 cursor reset 으로 사라짐.)
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        for seq in 0..200u64 {
            store.on_frame_all(a, 0, seq, vec![b'o']);
        }
        // epoch 1 새 스트림 seq 0 — reset 으로 cursor None 이 돼 새 낮은 seq 통과.
        let out = store.on_frame_all(a, 1, 0, b"NEW".to_vec());
        assert_eq!(
            collected(&out, 1),
            b"NEW",
            "epoch 전환 후 새 낮은 seq 통과(drop 0)"
        );
    }

    // ── 생명주기: 마지막 slot 빠지면 콘텐츠 drop, 재배정 시 빈 신설(§4) ──────────────────

    #[test]
    fn content_dropped_when_last_viewer_removed() {
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.subscribe_all(2, a);
        store.on_frame_all(a, 0, 0, b"x".to_vec());
        assert_eq!(store.buffered_agent_count(), 1);

        // 첫 slot 제거 → 아직 viewer 있음 → 콘텐츠 유지.
        assert!(!store.unsubscribe(&1), "아직 viewer 남음 → drop 안 함");
        assert_eq!(store.buffered_agent_count(), 1);
        assert!(store.agent_has_viewers(a));

        // 마지막 slot 제거 → 콘텐츠 drop(누수 0).
        assert!(store.unsubscribe(&2), "마지막 viewer 빠짐 → 콘텐츠 drop");
        assert_eq!(store.buffered_agent_count(), 0);
        assert!(!store.agent_has_viewers(a));
        assert_eq!(store.resubscribe_after_seq(a), None, "drop 후 버퍼 없음");
    }

    #[test]
    fn reassign_after_drop_creates_fresh_empty_buffer() {
        // 마지막 viewer 빠져 drop 된 뒤 재배정 → 빈 콘텐츠 신설(데몬 replay 로 다시 채움).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"old".to_vec());
        assert!(store.unsubscribe(&1)); // drop

        // 재배정 → 빈 신설(옛 콘텐츠 없음 → replay 0).
        let replay = store.subscribe_all(1, a);
        assert!(
            replay.is_empty(),
            "drop 후 재배정은 빈 버퍼(데몬 replay 가 채움)"
        );
        assert_eq!(store.buffered_agent_count(), 1);

        // 데몬이 FromOldest 로 새로 채우면 그 slot 이 받는다.
        let out = store.on_frame_all(a, 0, 0, b"fresh".to_vec());
        assert_eq!(collected(&out, 1), b"fresh");
    }

    #[test]
    fn unsubscribe_unknown_slot_is_false() {
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        assert!(!store.unsubscribe(&99), "없던 slot → false");
    }

    // ── buffered_agents 결정론 ────────────────────────────────────────────────────────

    #[test]
    fn buffered_agents_sorted_and_reflects_content() {
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, aid(3));
        store.subscribe_all(2, aid(1));
        store.subscribe_all(3, aid(2));
        let agents = store.buffered_agents();
        assert_eq!(agents, vec![aid(1), aid(2), aid(3)], "정렬돼 반환");
    }

    // ── 갭① mount-즉시-replay(agent 단위 subscribe_agent — TRD §2·수용기준 5) ─────────────

    #[test]
    fn subscribe_agent_replays_immediately_to_quiet_agent() {
        // ★조용한 agent mount 즉시 replay 나감★: frame 이 더는 안 와도 mount 시점 버퍼를 즉시 flush.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        // slot1 이 보는 동안 3건 도착(이후 agent 조용해짐 — 더는 frame 없음).
        store.subscribe_agent_all(a, &[1]);
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());
        store.on_frame_all(a, 0, 2, b"c".to_vec());

        // slot2 가 늦게 그 agent 를 배정받음(0→1 아님 — 이미 slot1 이 봄. 그러나 신규 slot 추가).
        // subscribe_agent 가 cursor 없는 slot2 만 신설 + 즉시 replay(frame 안 와도).
        let replay = store.subscribe_agent_all(a, &[1, 2]);
        assert_eq!(collected(&replay, 2), b"abc", "신규 slot2 즉시 전체 replay");
        assert!(
            collected(&replay, 1).is_empty(),
            "이미 보던 slot1 은 불가침(중복 replay 0)"
        );
    }

    #[test]
    fn subscribe_agent_no_duplicate_send_when_frame_follows() {
        // ★subscribe 후 frame 중복 전송 안 됨(정합 — deep 리뷰 급소)★: subscribe_agent 로 replay 받은
        //   직후 첫 frame 이 와도, 그 slot 은 *replay 로 이미 본 구간* 을 다시 받지 않는다.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_agent_all(a, &[1]);
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());

        // slot2 mount(0→1 아님이지만 신규 slot): replay = ab.
        let replay = store.subscribe_agent_all(a, &[1, 2]);
        assert_eq!(collected(&replay, 2), b"ab");

        // 직후 첫 frame(seq 2) → slot2 는 c 만(ab 중복 0), slot1 도 c 만.
        let out = store.on_frame_all(a, 0, 2, b"c".to_vec());
        assert_eq!(
            collected(&out, 2),
            b"c",
            "replay 직후 frame 은 신규분만(중복 0)"
        );
        assert_eq!(collected(&out, 1), b"c");
    }

    #[test]
    fn subscribe_agent_on_quiet_disconnected_buffer_is_not_blank() {
        // ★재연결 대기 중 새 창 즉시 replay(수용기준 5)★: 끊긴 상태(더 이상 frame 안 옴)에서도 버퍼가
        //   보존돼 있으면 새 slot 이 mount 시 즉시 그 버퍼를 받는다(빈 화면 0).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_agent_all(a, &[1]);
        store.on_frame_all(a, 0, 0, b"x".to_vec());
        store.on_frame_all(a, 0, 1, b"y".to_vec());
        // (여기서 "끊김" — 더 이상 on_frame 없음.) 새 창 slot2 가 그 agent 를 보기 시작.
        let replay = store.subscribe_agent_all(a, &[1, 2]);
        assert_eq!(
            collected(&replay, 2),
            b"xy",
            "끊긴 버퍼라도 새 창 mount 즉시 replay(빈 화면 0)"
        );
    }

    #[test]
    fn subscribe_agent_empty_buffer_replays_nothing() {
        // 콘텐츠가 빈 신규 agent(데몬 replay 아직 안 옴)에 subscribe_agent → 빈 snapshot + 콘텐츠 신설.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        let replay = store.subscribe_agent_all(a, &[1]);
        assert!(replay.is_empty());
        assert_eq!(
            store.buffered_agent_count(),
            1,
            "빈 콘텐츠 신설(데몬이 채움)"
        );
        // 데몬 FromOldest replay 가 on_frame 으로 도착하면 그 slot 이 받는다.
        let out = store.on_frame_all(a, 0, 0, b"z".to_vec());
        assert_eq!(collected(&out, 1), b"z");
    }

    // ── 정합: subscribe_agent → 직후 sync_viewers 가 cursor 를 되돌리지 않음 ──────────────

    #[test]
    fn sync_viewers_does_not_rewind_cursor_set_by_subscribe_agent() {
        // ★race 정합★: layout subscribe_agent(0→1 mount replay) 직후, 첫 frame 의 on_frame 이 부르는
        //   sync_viewers 가 그 slot cursor 를 None 으로 되돌려(재insert) 중복 replay 를 내면 안 된다.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        // slot1 이 먼저 보며 2건 쌓음.
        store.subscribe_agent_all(a, &[1]);
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());

        // slot2 가 0→1 델타로 mount → 즉시 ab replay 받고 cursor=Some(1) 로 advance.
        let replay = store.subscribe_agent_all(a, &[1, 2]);
        assert_eq!(collected(&replay, 2), b"ab");

        // 직후 첫 frame(seq 2) — on_frame 이 sync_viewers(router 파생 slot=[1,2])를 먼저 부른다고 모사.
        //   sync_viewers 는 slot2 cursor 가 *이미 있으니*(is_none() 가드) 재insert 안 함 → 되돌림 0.
        store.sync_viewers(a, &[1, 2]);
        let out = store.on_frame_all(a, 0, 2, b"c".to_vec());
        assert_eq!(
            collected(&out, 2),
            b"c",
            "sync_viewers 가 cursor 안 되돌림 → 신규분만(ab 중복 0)"
        );
    }

    // ── 갭② 생명주기 폐기(drop_agent — frame 도착 독립, TRD §4) ───────────────────────────

    #[test]
    fn drop_agent_drops_buffer_independent_of_frames() {
        // ★terminal+창닫기 후 버퍼 drop★: frame 이 더는 안 오는 terminal agent 도, 배정 해제(drop_agent)
        //   시 정상 폐기된다(frame 트리거에 안 묶임 — 누수 0).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_agent_all(a, &[1, 2]);
        store.on_frame_all(a, 0, 0, b"x".to_vec());
        assert_eq!(store.buffered_agent_count(), 1);
        assert!(store.agent_has_viewers(a));

        // (agent terminal — 이후 frame 0.) layout 이 두 창 모두에서 그 agent 를 떼어냄(1→0 델타).
        let dropped = store.drop_agent(a);
        assert!(dropped, "콘텐츠 들고 있다 폐기 → true");
        assert_eq!(store.buffered_agent_count(), 0, "frame 안 와도 버퍼 drop");
        assert!(!store.agent_has_viewers(a), "모든 cursor 제거됨");
        assert_eq!(store.resubscribe_after_seq(a), None, "drop 후 버퍼 없음");
    }

    #[test]
    fn drop_agent_on_unknown_is_noop_false() {
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        assert!(
            !store.drop_agent(aid(99)),
            "콘텐츠 없던 agent → false(no-op)"
        );
    }

    #[test]
    fn drop_agent_then_reassign_creates_fresh_buffer() {
        // drop_agent 폐기 후 재배정(다시 0→1) → 빈 신설(데몬 replay 가 채움).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_agent_all(a, &[1]);
        store.on_frame_all(a, 0, 0, b"old".to_vec());
        assert!(store.drop_agent(a));
        assert_eq!(store.buffered_agent_count(), 0);

        // 재배정 → 빈 콘텐츠 신설(옛 콘텐츠 없음 → replay 0).
        let replay = store.subscribe_agent_all(a, &[1]);
        assert!(replay.is_empty(), "drop 후 재배정은 빈 버퍼");
        // 데몬이 FromOldest 로 새로 채우면 그 slot 이 받는다.
        let out = store.on_frame_all(a, 0, 0, b"fresh".to_vec());
        assert_eq!(collected(&out, 1), b"fresh");
    }

    // ── 근원2: cursor advance ⟺ Channel delivery(deliverable 게이트 + resubscribe_slot) ──────

    /// (slot, bytes) snapshot 에서 어떤 slot 이라도 등장하는지(전달 여부 판정).
    fn slot_received(out: &[(u32, Vec<u8>)], slot: u32) -> bool {
        out.iter().any(|(s, _)| *s == slot)
    }

    #[test]
    fn undeliverable_slot_does_not_advance_cursor() {
        // ★근원2 핵심★: Channel 미등록(deliverable 에 없는) slot 은 frame 도착해도 snapshot 에 안 담기고
        //   cursor advance 도 안 한다 — 전달 못 한 구간을 advance 하면 이후 영구 유실되기 때문.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        // slot1 은 Channel 등록(deliverable), slot2 는 layout 배정만 됐고 webview 미mount(미등록).
        let deliverable: HashSet<u32> = [1u32].into_iter().collect();
        store.subscribe_all(1, a);
        // ★slot2 = 미등록 배정★: deliverable={1} 로 subscribe → cursor 만 None 신설(membership), advance 0.
        store.subscribe(2, a, &deliverable);

        // frame 도착: deliverable = {slot1} 뿐(slot2 의 Channel 은 아직 없음).
        let out = store.on_frame(a, 0, 0, b"x".to_vec(), &deliverable);
        assert!(slot_received(&out, 1), "등록된 slot1 은 전달");
        assert!(
            !slot_received(&out, 2),
            "미등록 slot2 는 전달 안 됨(snapshot 에 없음)"
        );
        // ★slot2 cursor 가 advance 안 됐어야★: 콘텐츠엔 seq 0 이 있지만 slot2 는 아직 안 읽음(None).
        assert_eq!(
            store.cursor_for_test(&2),
            Some(None),
            "미전달 slot2 cursor 는 None 유지(전진 금지)"
        );
        // slot1 은 seq 0 까지 읽음.
        assert_eq!(store.cursor_for_test(&1), Some(Some(0)));
    }

    #[test]
    fn stalled_undeliverable_slot_recovers_full_on_mount() {
        // ★근원2 무손실 복구★: 미등록 slot 이 frame 들을 못 받고 cursor 가 멈춰 있다가, 그 창 mount 시
        //   resubscribe_slot(fresh 리셋+전체 replay)이 처음부터 무손실로 채운다(빈 화면 0).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        let deliverable: HashSet<u32> = [1u32].into_iter().collect();
        store.subscribe_all(1, a); // slot1 등록(deliverable)

        // ★slot2 = 배정만(미mount/미등록)★: deliverable={1} 로 subscribe → cursor 엔트리는 신설(membership)
        //   되지만 값은 None(advance 게이트 — 5차 FIX). 4차 땐 게이트 부재로 우연히 None 이었던 게, 이제
        //   subscribe 단계부터 일관 게이트된다.
        store.subscribe(2, a, &deliverable);

        // slot1 만 받는 동안 3건 도착(slot2 는 deliverable 아님 → cursor 멈춤).
        store.on_frame(a, 0, 0, b"a".to_vec(), &deliverable);
        store.on_frame(a, 0, 1, b"b".to_vec(), &deliverable);
        store.on_frame(a, 0, 2, b"c".to_vec(), &deliverable);
        assert_eq!(
            store.cursor_for_test(&2),
            Some(None),
            "slot2 cursor 멈춤(None) — 신설은 됐으나 미전달"
        );

        // slot2 webview 가 mount → resubscribe_slot 이 fresh 리셋 + 버퍼 전체(abc) replay.
        let replay = store.resubscribe_slot_all(2, a);
        assert_eq!(
            collected(&replay, 2),
            b"abc",
            "mount 시 멈춰있던 slot 도 전체 무손실 replay(빈 화면 0)"
        );
        // 이제 slot2 도 deliverable → 다음 frame 은 둘 다 신규분만.
        let deliverable2: HashSet<u32> = [1u32, 2].into_iter().collect();
        let out = store.on_frame(a, 0, 3, b"d".to_vec(), &deliverable2);
        assert_eq!(collected(&out, 1), b"d");
        assert_eq!(collected(&out, 2), b"d", "slot2 도 d 만(abc 중복 0)");
    }

    #[test]
    fn normal_mount_assignment_then_registration_replays_once() {
        // ★FIX-2(4차) 정상 mount = 전체 replay 1회★: 배정 트리거(subscribe — cursor 없을 때만 신설+replay)가
        //   먼저 replay 를 내고, 직후 등록 트리거(resubscribe_slot)가 와도 *그 사이 cursor 가 이미 advance* 돼
        //   reload 면 fresh, 정상 mount 면 같은 구간이지만 — Rust 가 2회 전체 replay 를 *연속으로* 내지 않게
        //   배정/등록 역할을 분리했다. 여기선 store 레벨에서 "subscribe 가 cursor 를 신설·advance 했으면, 그
        //   직후 다시 subscribe(배정 재트리거)는 불가침이라 replay 0" 을 박는다(중복 1회 제거).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.on_frame_all(a, 0, 0, b"a".to_vec()); // viewer 0 이라 전달 0, 콘텐츠만.
        store.on_frame_all(a, 0, 1, b"b".to_vec());
        // 배정 트리거(layout 델타): subscribe — cursor 없으니 신설 + 전체 ab replay.
        let assign = store.subscribe_all(1, a);
        assert_eq!(
            collected(&assign, 1),
            b"ab",
            "배정 트리거가 첫 전체 replay 1회"
        );
        // 등록 트리거(subscribe_output)가 *배정과 같은 mount* 에서 또 와도, 배정 트리거(subscribe)를
        //   재호출하는 경로는 불가침이라 0(아래). 즉 정상 mount 에서 전체 replay 가 연속 2회 안 나간다.
        let assign_again = store.subscribe_all(1, a);
        assert!(
            assign_again.is_empty(),
            "★배정 트리거 재호출은 불가침(cursor 존재) → 중복 replay 0★"
        );
    }

    #[test]
    fn reload_via_registration_replays_fresh_even_after_assignment() {
        // ★FIX-2 reload = fresh 1회★: 배정 트리거(subscribe)가 신설한 뒤 webview reload(Channel 교체)가
        //   오면, 등록 트리거(resubscribe_slot)는 stale cursor 를 무시하고 전체 replay 를 다시 준다(빈 화면 0).
        //   배정(불가침) ↔ 등록(fresh)의 역할 분리가 정확히 이 차이를 만든다.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());
        let assign = store.subscribe_all(1, a); // 배정: 신설 + ab
        assert_eq!(collected(&assign, 1), b"ab");
        // reload: 등록 트리거는 resubscribe_slot(fresh) — cursor 있어도 None 리셋 후 전체 ab 재replay.
        let reload = store.resubscribe_slot_all(1, a);
        assert_eq!(
            collected(&reload, 1),
            b"ab",
            "★등록 트리거(resubscribe_slot)는 reload 시 fresh 전체 replay★"
        );
    }

    #[test]
    fn resubscribe_slot_resets_stale_cursor_for_reload() {
        // ★webview reload 빈 화면 차단★: 이미 진도가 나간 slot 이라도 resubscribe_slot 은 cursor 를 None 으로
        //   강제 리셋 후 전체 replay 한다(같은 slot key 로 Channel 이 *교체*되는 reload — 새 xterm 은 빈 화면).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        let deliverable: HashSet<u32> = [1u32].into_iter().collect();
        store.on_frame(a, 0, 0, b"a".to_vec(), &deliverable);
        store.on_frame(a, 0, 1, b"b".to_vec(), &deliverable);
        // slot1 은 seq 1 까지 읽음(이어보기 중).
        assert_eq!(store.cursor_for_test(&1), Some(Some(1)));

        // webview reload — 같은 slot key 로 Channel 재등록 → resubscribe_slot. ★불가침(subscribe)이 아니라
        //   강제 리셋★: subscribe 면 cursor 가 있어 빈 replay(reload 빈 화면). resubscribe_slot 은 전체 ab.
        let replay = store.resubscribe_slot_all(1, a);
        assert_eq!(
            collected(&replay, 1),
            b"ab",
            "reload 시 stale cursor 무시하고 전체 replay(빈 화면 0)"
        );
        // 대조: subscribe 였다면 빈 replay 였을 것(불가침). 그 차이가 reload 버그의 핵심.
    }

    // ── FIX-1: 고아 cursor sweep(drop_slots full 누수 reconcile) ──────────────────────────

    #[test]
    fn sweep_orphans_removes_cursors_not_in_keep() {
        // ★FIX-1★: drop_slots 가 채널 full 로 silent drop 돼 cursor 가 잔존하면, connect 진입 sweep 이
        //   router 현재 집합(keep)에 없는 cursor 를 제거하고 마지막이면 콘텐츠도 drop 한다.
        let a = aid(1);
        let b = aid(2);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        // slot1→a, slot2→a, slot3→b 가 보던 상태(콘텐츠 a·b 둘 다 있음).
        store.subscribe_all(1, a);
        store.subscribe_all(2, a);
        store.subscribe_all(3, b);
        store.on_frame_all(a, 0, 0, b"x".to_vec());
        store.on_frame_all(b, 0, 0, b"y".to_vec());
        assert_eq!(store.buffered_agent_count(), 2);

        // layout 이 slot2(a)·slot3(b)를 떼어냈으나 drop_slots 가 full 로 유실됐다고 가정 → cursor 잔존.
        //   현재 router 에 실재하는 건 slot1(a) 뿐(keep).
        let keep: HashSet<u32> = [1u32].into_iter().collect();
        let removed = store.sweep_orphans(&keep);
        assert_eq!(removed, 2, "keep 에 없는 slot2·slot3 cursor 제거");
        // slot1(a)은 보존 → a 콘텐츠 유지, b 는 마지막 viewer(slot3) 빠져 콘텐츠 drop.
        assert!(
            store.agent_has_viewers(a),
            "keep 의 slot1 보존 → a viewer 유지"
        );
        assert!(!store.agent_has_viewers(b), "b 는 고아 → viewer 0");
        assert_eq!(
            store.buffered_agent_count(),
            1,
            "b 콘텐츠 drop(마지막 viewer 빠짐)"
        );
        assert_eq!(store.resubscribe_after_seq(b), None, "b 버퍼 폐기");
        assert_eq!(store.resubscribe_after_seq(a), Some(0), "a 버퍼 보존");
    }

    #[test]
    fn sweep_orphans_noop_when_all_kept() {
        // keep 이 현재 cursor 전부를 포함하면 아무것도 안 지운다(정합 상태 — 0 반환).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.subscribe_all(2, a);
        let keep: HashSet<u32> = [1u32, 2].into_iter().collect();
        assert_eq!(store.sweep_orphans(&keep), 0, "전부 keep → 제거 0");
        assert_eq!(store.buffered_agent_count(), 1);
        assert!(store.agent_has_viewers(a));
    }

    // ── FIX-1(4차): 양방향 reconcile(유실 replay 복구 + 고아 sweep) ─────────────────────────

    #[test]
    fn reconcile_recovers_lost_replay_for_kept_slot_without_cursor() {
        // ★FIX-1 (a) 유실 replay 복구★: router 엔 배정됐으나(keep) cursor 가 없는 slot(replay_slots full
        //   drop 으로 신설 못 함)을, reconcile 이 fresh 신설 + 그 시점 버퍼 전체 replay 로 복구한다.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a); // slot1 은 정상 신설(cursor 존재)
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());
        // slot2 는 router 에 배정됐다고 가정하나 cursor 미신설(유실된 replay_slots) — keep 에는 있다.
        let keep = [(1u32, a), (2u32, a)];
        let (replay, removed) = store.reconcile_slots_all(&keep);
        assert_eq!(removed, 0, "둘 다 keep → 고아 sweep 0");
        assert_eq!(
            collected(&replay, 2),
            b"ab",
            "cursor 없던 slot2 를 reconcile 이 fresh 전체 replay 로 복구"
        );
        assert!(
            collected(&replay, 1).is_empty(),
            "★멱등★: cursor 있던 slot1 은 불가침(중복 replay 0)"
        );
        // 복구 후 다음 frame 은 둘 다 신규분만(slot2 도 ab 중복 0).
        let out = store.on_frame_all(a, 0, 2, b"c".to_vec());
        assert_eq!(collected(&out, 1), b"c");
        assert_eq!(collected(&out, 2), b"c");
    }

    #[test]
    fn reconcile_is_idempotent_no_duplicate_replay() {
        // ★FIX-1 멱등★: 정상 cursor 인 slot 만 있을 때 reconcile 을 여러 번 돌려도 replay 0(중복 send 차단).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"x".to_vec());
        let keep = [(1u32, a)];
        let (r1, _) = store.reconcile_slots_all(&keep);
        assert!(r1.is_empty(), "이미 cursor 있는 slot → reconcile replay 0");
        let (r2, _) = store.reconcile_slots_all(&keep);
        assert!(r2.is_empty(), "재호출도 멱등 — replay 0");
    }

    #[test]
    fn reconcile_sweeps_orphan_and_recovers_in_one_pass() {
        // ★FIX-1 양방향 동시★: keep 에 없는 cursor(고아)는 sweep, keep 에 있는데 cursor 없는 slot 은 replay.
        let a = aid(1);
        let b = aid(2);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a); // slot1→a 정상
        store.subscribe_all(3, b); // slot3→b 정상(곧 고아가 될 것)
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(b, 0, 0, b"b".to_vec());
        // router 현재: slot1(a) 유지 + slot2(a) 신규(cursor 없음). slot3(b)는 떼어냄(keep 에 없음=고아).
        let keep = [(1u32, a), (2u32, a)];
        let (replay, removed) = store.reconcile_slots_all(&keep);
        assert_eq!(removed, 1, "keep 에 없는 slot3 고아 제거");
        assert!(
            !store.agent_has_viewers(b),
            "b 는 마지막 viewer(slot3) 빠져 폐기"
        );
        assert_eq!(
            collected(&replay, 2),
            b"a",
            "slot2 신규는 a 버퍼 전체 replay 복구"
        );
        assert!(collected(&replay, 1).is_empty(), "slot1 불가침(중복 0)");
    }

    #[test]
    fn reconcile_empty_keep_sweeps_all() {
        // keep 이 비면 모든 cursor 가 고아 → 전부 sweep(콘텐츠도 폐기). replay 0.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"x".to_vec());
        let (replay, removed) = store.reconcile_slots_all(&[]);
        assert!(replay.is_empty(), "keep 빔 → 신규 replay 0");
        assert_eq!(removed, 1, "모든 cursor 고아 제거");
        assert_eq!(
            store.buffered_agent_count(),
            0,
            "마지막 viewer 빠져 콘텐츠 drop"
        );
    }

    #[test]
    fn deliverable_gate_does_not_block_content_append() {
        // ★축 A 비회귀★: deliverable 게이트는 *전달/advance* 만 막고, 콘텐츠 append(재구독 after_seq 대비)는
        //   항상 한다 — 미등록 slot 만 있는 agent 도 콘텐츠는 쌓여 재연결 무손실(축 A) 유지.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        let empty: HashSet<u32> = HashSet::new(); // 아무것도 deliverable 아님.
        store.subscribe(1, a, &empty); // 배정만(미등록) — cursor None 신설, advance 0.
        store.on_frame(a, 0, 0, b"x".to_vec(), &empty);
        store.on_frame(a, 0, 1, b"y".to_vec(), &empty);
        // 전달은 0 이지만 콘텐츠는 쌓였다 → 축 A 재구독 기준 = 버퍼 최신(1).
        assert_eq!(
            store.resubscribe_after_seq(a),
            Some(1),
            "미전달이어도 콘텐츠 append(축 A after_seq 유지)"
        );
        assert_eq!(
            store.cursor_for_test(&1),
            Some(None),
            "전달 0 → cursor 전진 0"
        );
    }

    // ── 5차 FIX: mount/reconcile 경로 deliverable 게이트(정합 갭 — opus FIX + Codex BLOCK 수렴) ──

    #[test]
    fn subscribe_undeliverable_creates_membership_but_does_not_advance() {
        // ★5차 핵심(subscribe 경로)★: 배정 트리거(subscribe)가 미등록(deliverable 밖) slot 에 불리면 cursor
        //   엔트리는 신설(membership)하되 advance/replay 는 안 한다(값 None) — 콘텐츠가 *이미 있어도* 그렇다.
        //   4차는 콘텐츠 있는 slot 을 deliverable 무관하게 신설 즉시 advance 해, 그 snapshot 이 flush registry
        //   miss 로 스킵되는 동안 cursor 만 전진 → 영구 유실(이후 reconcile 불가침 오판). 게이트로 차단.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        // 콘텐츠를 먼저 쌓는다(slot1 이 보며 ab — slot2 mount 전에 버퍼 존재).
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());

        // slot2 를 미등록(deliverable={1}, slot2 없음)으로 배정 → cursor 신설되나 값 None(advance 0).
        let deliverable: HashSet<u32> = [1u32].into_iter().collect();
        let replay = store.subscribe(2, a, &deliverable);
        assert!(
            replay.is_empty(),
            "미등록 slot 은 콘텐츠 있어도 replay 0(전달 못 함)"
        );
        assert_eq!(
            store.cursor_for_test(&2),
            Some(None),
            "★cursor 엔트리는 신설(membership)되나 값 None(advance 게이트)★"
        );
    }

    #[test]
    fn reconcile_recovers_membership_only_slot_when_it_becomes_deliverable() {
        // ★5차 BLOCK 핵심(Codex)★: 배정만 됐다가(cursor 값 None — 미전달) 등록 트리거마저 채널 full 로 drop 된
        //   slot 을, reconcile 이 "엔트리 있음=완료" 로 오판해 건너뛰면 영구 빈 화면. 5차 가드는 판정을 *값*
        //   기준으로 바꿔 "값 None + deliverable" 을 복구 대상으로 잡는다 → 전체 무손실 replay.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());

        // slot2 = 배정만(미등록) → cursor 엔트리 있으나 값 None(미전달).
        let undeliverable: HashSet<u32> = [1u32].into_iter().collect();
        store.subscribe(2, a, &undeliverable);
        assert_eq!(
            store.cursor_for_test(&2),
            Some(None),
            "slot2 값 None(미전달)"
        );

        // 그 창 webview 가 mount(slot2 Channel 등록) → 이제 deliverable={1,2}. 등록 트리거 replay 가 유실됐다고
        //   가정하고 reconcile 이 따라잡는다(handoff resync/재연결 진입 경로).
        let deliverable: HashSet<u32> = [1u32, 2].into_iter().collect();
        let keep = [(1u32, a), (2u32, a)];
        let (replay, removed) = store.reconcile_slots(&keep, &deliverable);
        assert_eq!(removed, 0, "둘 다 keep → 고아 0");
        assert_eq!(
            collected(&replay, 2),
            b"ab",
            "★값 None + deliverable 인 slot2 를 reconcile 이 전체 무손실 replay 로 복구(영구 빈 화면 차단)★"
        );
        assert!(
            collected(&replay, 1).is_empty(),
            "값 Some 인 slot1 은 불가침(중복 0)"
        );
        // 복구 후 다음 frame 은 둘 다 신규분만(slot2 도 ab 중복 0).
        let out = store.on_frame_all(a, 0, 2, b"c".to_vec());
        assert_eq!(collected(&out, 1), b"c");
        assert_eq!(collected(&out, 2), b"c");
    }

    #[test]
    fn reconcile_does_not_replay_undeliverable_membership_slot() {
        // ★갈림길 안전성★: 값 None 이라도 *미deliverable*(아직 미mount) 이면 reconcile 이 건드리지 않는다
        //   (membership 유지, replay 0, advance 0). deliverable 무관하게 replay 하면 전달 못 할 구간을 advance
        //   해 다시 영구 유실 — 그 창은 mount 시 등록 트리거가 채운다(빈 화면 0 은 등록 트리거가 보장).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"a".to_vec());

        // slot2 배정만(미등록). reconcile 의 deliverable 에도 slot2 없음(여전히 미mount).
        let undeliverable: HashSet<u32> = [1u32].into_iter().collect();
        store.subscribe(2, a, &undeliverable);
        let keep = [(1u32, a), (2u32, a)];
        let (replay, removed) = store.reconcile_slots(&keep, &undeliverable);
        assert_eq!(removed, 0, "keep 에 둘 다 있음 → 고아 0");
        assert!(
            collected(&replay, 2).is_empty(),
            "★미deliverable slot2 는 reconcile 이 replay 안 함(membership 유지)★"
        );
        assert_eq!(
            store.cursor_for_test(&2),
            Some(None),
            "★advance 0 — 값 None 유지(전달 못 할 구간 전진 금지)★"
        );
    }

    #[test]
    fn deliverable_slot_subscribe_replays_exactly_once() {
        // ★정상 mount = 전체 replay 정확히 1회(중복 0)★: deliverable slot 에 subscribe → 전체 replay 1회,
        //   재호출(배정 재트리거)은 불가침이라 0. on_frame 직후에도 신규분만(replay 구간 중복 0).
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"a".to_vec());
        store.on_frame_all(a, 0, 1, b"b".to_vec());

        // slot2 정상 mount(deliverable) → ab 1회.
        let deliverable: HashSet<u32> = [1u32, 2].into_iter().collect();
        let r1 = store.subscribe(2, a, &deliverable);
        assert_eq!(collected(&r1, 2), b"ab", "정상 mount = 전체 replay 1회");
        // 재호출 불가침(cursor 있음) → 0.
        let r2 = store.subscribe(2, a, &deliverable);
        assert!(r2.is_empty(), "★배정 재트리거는 불가침 → 중복 replay 0★");
        // 직후 frame 은 신규분만.
        let out = store.on_frame_all(a, 0, 2, b"c".to_vec());
        assert_eq!(
            collected(&out, 2),
            b"c",
            "replay 직후 frame 은 신규분만(중복 0)"
        );
    }

    #[test]
    fn reconcile_idempotent_does_not_touch_delivered_cursor() {
        // ★reconcile 멱등(값 Some 불가침)★: 정상 전달 중(값 Some)인 slot 은 deliverable 이어도 reconcile 을
        //   여러 번 돌려도 절대 다시 replay 안 한다(on_frame 단독 책임). 4차 멱등 가드의 정신을 값 기준으로 유지.
        let a = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, a);
        store.on_frame_all(a, 0, 0, b"x".to_vec()); // slot1 값 Some(0)
        assert_eq!(store.cursor_for_test(&1), Some(Some(0)));

        let deliverable: HashSet<u32> = [1u32].into_iter().collect();
        let keep = [(1u32, a)];
        let (r1, _) = store.reconcile_slots(&keep, &deliverable);
        assert!(r1.is_empty(), "값 Some slot → reconcile replay 0");
        let (r2, _) = store.reconcile_slots(&keep, &deliverable);
        assert!(r2.is_empty(), "재호출도 멱등 — replay 0");
        // 값 보존(되돌림 0).
        assert_eq!(store.cursor_for_test(&1), Some(Some(0)), "cursor 값 불가침");
    }

    // ── S15 B8: opaque frame replay(tag0/tag1 무관 byte-identical 보존) ────────────────────────
    //
    // ★왜 이 store 가 opaque replay 대상인가★: 클라 relay(src-tauri connection.rs)는 데몬 binary frame 을
    // decode_frame 으로 헤더(agent_id/epoch/seq)만 뽑아 on_frame 의 태깅·cursor 인덱스로 쓰고, **저장·replay
    // 단위는 원본 frame bytes(헤더 포함) 전체**다(connection.rs 주석 §1·§2 — 프론트 tauriTransport 가 원본
    // frame 을 다시 decode 하므로). 즉 이 store 의 bytes 는 payload 가 아니라 **opaque frame**이라, tag 종류
    // (tag0 terminal / tag1 structured, ADR-0045)와 무관하게 넣은 그대로 무손실 반환해야 한다.
    //
    // ★dev-dep protocol 로 실제 codec 사용★: frame 을 손으로 조립하지 않고 encode_terminal_frame/
    // encode_structured_frame 으로 만들어(레이아웃 표류 시 이 테스트도 함께 잡힘) codec 계약과 정합시킨다.
    // protocol 은 core 를 의존 안 하므로 순환 아님(dev-only, ADR-0003 런타임 격리는 유지).
    use engram_dashboard_protocol::{encode_structured_frame, encode_terminal_frame};

    /// 테스트용 결정적 16바이트 agent_id(protocol AgentId=uuid).
    fn puuid(n: u128) -> engram_dashboard_protocol::AgentId {
        engram_dashboard_protocol::AgentId::from_u128(n)
    }

    /// on_frame 이 반환한 snapshot 에서 한 slot 이 받은 frame bytes 들을 순서대로 모은다(frame 단위 보존).
    fn frames_for(out: &[(u32, Vec<u8>)], slot: u32) -> Vec<Vec<u8>> {
        out.iter()
            .filter(|(s, _)| *s == slot)
            .map(|(_, b)| b.clone())
            .collect()
    }

    #[test]
    fn tag1_structured_frame_replays_byte_identical() {
        // tag1 frame bytes(헤더+JSON payload)를 통째로 on_frame 에 넣고 → read/replay 가 헤더 포함 원본
        //   frame 과 byte-identical 로 나오는지. store 는 payload 스키마를 모르는 opaque 버퍼임을 검증.
        let store_aid = aid(1);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, store_aid);

        // 실제 codec 으로 tag1 frame 조립(payload=self-describing JSON 흉내 — 내용은 opaque).
        let payload = br#"{"type":"TextDelta","text":"hi","turn_id":null,"message_id":null}"#;
        let frame = encode_structured_frame(puuid(1), /*epoch*/ 0, /*seq*/ 0, payload);

        // live 전달: on_frame 의 bytes = 원본 frame 전체(클라 relay 와 동일 — 헤더 포함 저장).
        let out = store.on_frame_all(store_aid, 0, 0, frame.clone());
        let got = frames_for(&out, 1);
        assert_eq!(got.len(), 1, "slot1 이 frame 1건 받음");
        assert_eq!(got[0], frame, "tag1 frame 이 byte-identical 로 전달");

        // 늦게 mount 한 slot2 replay 도 동일 frame 을 byte-identical 로.
        let replay = store.subscribe_all(2, store_aid);
        let r = frames_for(&replay, 2);
        assert_eq!(r.len(), 1, "slot2 replay frame 1건");
        assert_eq!(r[0], frame, "replay 도 원본 frame byte-identical");
        // 첫 바이트 tag=1 이 보존됐는지(store 가 tag 를 안 건드림).
        assert_eq!(r[0][0], 1u8, "tag1 바이트 보존");
    }

    #[test]
    fn mixed_tag0_tag1_buffer_preserves_order_and_bytes() {
        // tag0(terminal)·tag1(structured) 이 섞인 버퍼에서 cursor replay 가 seq 순서·내용·tag 를
        //   전부 보존하는지. store 는 두 tag 를 구분하지 않는 opaque frame 버퍼임을 확인.
        let store_aid = aid(7);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, store_aid);

        let f0 = encode_terminal_frame(puuid(7), 0, 0, b"raw-bytes");
        let f1 = encode_structured_frame(puuid(7), 0, 1, br#"{"type":"Usage"}"#);
        let f2 = encode_terminal_frame(puuid(7), 0, 2, b"more");

        // 순차 live 전달(seq 0,1,2). slot1 은 매 frame 을 원본 그대로 받는다.
        assert_eq!(
            frames_for(&store.on_frame_all(store_aid, 0, 0, f0.clone()), 1),
            vec![f0.clone()]
        );
        assert_eq!(
            frames_for(&store.on_frame_all(store_aid, 0, 1, f1.clone()), 1),
            vec![f1.clone()]
        );
        assert_eq!(
            frames_for(&store.on_frame_all(store_aid, 0, 2, f2.clone()), 1),
            vec![f2.clone()]
        );

        // 새 slot2 가 mount → 버퍼 전체를 seq 순서로 replay(tag0,tag1,tag0 혼합 그대로).
        let replay = frames_for(&store.subscribe_all(2, store_aid), 2);
        assert_eq!(
            replay,
            vec![f0.clone(), f1.clone(), f2.clone()],
            "혼합 tag 버퍼가 seq 순서·byte-identical 로 replay"
        );
        // tag 바이트 보존(0,1,0) — store 가 tag 를 해석/변형하지 않음.
        assert_eq!(
            replay.iter().map(|f| f[0]).collect::<Vec<_>>(),
            vec![0u8, 1u8, 0u8],
            "각 frame 의 tag 바이트 보존"
        );
    }

    #[test]
    fn tag1_cursor_resume_after_partial_read_is_lossless() {
        // cursor 가 이미 앞선 tag1 frame 을 읽은 뒤, 새 tag1 frame 만 무손실로 이어받는지(축 B cursor).
        let store_aid = aid(9);
        let mut store: OutputViewStore<u32> = OutputViewStore::new();
        store.subscribe_all(1, store_aid);

        let f0 = encode_structured_frame(puuid(9), 0, 0, br#"{"type":"MessageDone"}"#);
        store.on_frame_all(store_aid, 0, 0, f0.clone()); // slot1 cursor → seq0

        // 새 창 slot2 는 mount 시 f0 전체 replay(cursor=None).
        let r2_first = frames_for(&store.subscribe_all(2, store_aid), 2);
        assert_eq!(r2_first, vec![f0.clone()], "mount replay = f0");

        // 다음 tag1 frame(seq1) → 두 slot 모두 f1 만(이미 본 f0 재전송 없음).
        let f1 = encode_structured_frame(puuid(9), 0, 1, br#"{"type":"Error","message":"e"}"#);
        let out = store.on_frame_all(store_aid, 0, 1, f1.clone());
        assert_eq!(
            frames_for(&out, 1),
            vec![f1.clone()],
            "slot1 은 f1 만(무중복)"
        );
        assert_eq!(
            frames_for(&out, 2),
            vec![f1.clone()],
            "slot2 도 f1 만(무중복)"
        );
    }
}
