//! 스트림 살림 — ★순수 층★. 세대 대조 · 순번 중복 제거 · 구멍 검출 · 「유효 하한」.
//!
//! ★이 파일에는 tokio 도 소켓도 워크스페이스 crate 도 없다★ — 게이트가 그것을 잰다(정본 =
//! `lib.rs` 헤더). 시간은 **인자로** 받는다(`machine.rs` 와 같은 idiom).
//!
//! ★들어오는 쪽만 본다★ — 나가는 쪽 스트림(데몬이 순번을 찍는 쪽)은 이 crate 밖이다.
//!
//! ## 재요청과 그 답을 짝지어야 하는 이유 (되살리지 마라)
//!
//! **1판**은 「재요청을 냈다」를 참/거짓 하나로만 들고 있었다. 그러면 상대가 재요청을 받기 전에 이미
//! 띄워 놓은 조각(파이프라이닝 — 우리 출력 hot path 가 정확히 그렇게 한다)이 도착하는 순간 그것을
//! 「재요청에 대한 답이 또 구멍이다」로 읽어 잘림으로 접었고, 그 뒤 상대가 실제로 보낸 재생분은 전부
//! 「지나간 번호」가 되어 버려졌다.
//!
//! **2판**은 그 조각을 **버렸다**. 그것이 더 나빴다 — ★되감기 버퍼가 넘친 데몬은 정의상 `after+1` 을
//! 못 준다★(그게 잘림의 **주 경로**다). 그 경우 창이 닫힐 때까지 도착한 것이 사건도 배달도 없이
//! 사라지고, 그러고 나서 올리는 `oldest_valid` 는 **창 길이만큼 늦은 번호**였다. 즉 「버리고 값도
//! 틀리게 신고」였다.
//!
//! **지금**은 붙들어 둔다. 재요청의 답을 기다리는 동안 도착한 순서 밖 조각은 **유계 버퍼**에 담기고,
//! 창이 닫히면(시한 초과 · 버퍼 포화 · 스트림 축출) **순서대로 내보내면서** 그 첫 번호를
//! `oldest_valid` 로 올린다. 그래서 그 값이 참이 된다 — 우리가 실제로 그 번호부터 배달하기 때문이다.
//! 답이 제대로 오면(=`after+1` 이 도착) 붙든 것은 버린다: 상대의 재생분이 그 구간을 다시 실어 온다.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

/// 스트림 조각 하나의 표식.
///
/// ★`generation` 은 **화신 표식**이고 [`crate::Generation`](연결 세대)과 다른 것이다★ — 비교는
/// **일치/불일치만** 하고 대소로 "더 새 것" 을 유도하지 않는다(ADR-0163). 폭도 다르다(u32 대 u64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamMark<K> {
    pub key: K,
    pub generation: u32,
    pub seq: u64,
}

/// 조각 하나를 보고 내린 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict<M> {
    /// 이어진다 — 그대로 올린다.
    Deliver(M),
    /// 이미 지나간 순번(또는 이미 붙들어 둔 번호) — 버린다.
    Duplicate,
    /// 화신이 갈렸다 — 순번 대조를 초기화하고 올린다. `discarded` = 옛 화신 몫으로 붙들고 있다가
    /// 버린 조각 수(그 번호들은 새 화신에서 뜻이 없다).
    GenerationChanged {
        from: u32,
        to: u32,
        msg: M,
        discarded: usize,
    },
    /// 구멍. ★이 조각은 **붙들어 둔다**★ — 터미널 바이트를 순서 밖으로 올리면 ANSI 파서 상태가
    /// 어긋나 화면이 망가진다. `resume_from` 뒤부터 다시 청한다.
    Gap {
        expected: u64,
        got: u64,
        resume_from: u64,
    },
    /// 이미 낸 재요청의 답을 기다리는 동안 도착 — 붙들어 둔다. **유실이 아니다.**
    Held { awaiting_after: u64 },
    /// 창이 닫혔다 — 붙든 것을 순서대로 내보내고 이어 간다.
    ///
    /// ★`oldest_valid` 는 `flushed` 의 첫 번호다★ — 그 번호부터 실제로 배달하므로 참이다.
    Truncated { oldest_valid: u64, flushed: Vec<M> },
}

impl<M> Verdict<M> {
    /// 위로 올릴 조각들.
    pub fn into_delivered(self) -> Vec<M> {
        match self {
            Verdict::Deliver(msg) => vec![msg],
            Verdict::GenerationChanged { msg, .. } => vec![msg],
            Verdict::Truncated { flushed, .. } => flushed,
            Verdict::Duplicate | Verdict::Gap { .. } | Verdict::Held { .. } => Vec::new(),
        }
    }
}

/// 시한이 지난 재요청 하나가 접힌 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation<K, M> {
    pub key: K,
    pub generation: u32,
    pub oldest_valid: u64,
    pub flushed: Vec<M>,
}

/// 자리를 만들려고 잊은 스트림 하나.
///
/// ★조용히 사라지지 않는다★ — 붙들고 있던 조각은 `flushed` 로 나오고, 부르는 쪽이 사건으로 신고한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evicted<K, M> {
    pub key: K,
    pub generation: u32,
    pub last_seq: u64,
    /// 붙들고 있던 조각들. 비어 있지 않으면 `oldest_valid` 가 그 첫 번호다.
    pub flushed: Vec<M>,
    pub oldest_valid: Option<u64>,
}

/// 내보낸 재요청 하나.
#[derive(Debug)]
struct Resume<M> {
    /// 이 번호 **뒤**부터 달라고 청했다. 답의 첫 조각은 `after + 1` 이다.
    after: u64,
    /// 언제 청했나. 재연결 뒤 다시 낼 때 갱신된다.
    asked_at: Instant,
    /// 답을 기다리는 동안 도착한 순서 밖 조각들. 창이 닫히면 이 순서대로 나간다.
    held: BTreeMap<u64, M>,
}

#[derive(Debug)]
struct Track<M> {
    generation: u32,
    last_seq: u64,
    resume: Option<Resume<M>>,
}

impl<M> Track<M> {
    fn drain_held(&mut self) -> (Vec<M>, Option<u64>) {
        match self.resume.take() {
            Some(resume) => {
                let oldest = resume.held.keys().next().copied();
                let flushed: Vec<M> = resume.held.into_values().collect();
                (flushed, oldest)
            }
            None => (Vec::new(), None),
        }
    }
}

/// 스트림마다 「마지막 세대 · 마지막 순번 · 내보낸 재요청」을 쥔다.
///
/// ★연결이 끊겨도 지우지 않는다★ — 다시 붙었을 때 그 마지막 순번이 곧 이어받기 지점이라, 지우면
/// 재연결이 항상 처음부터 받는 것이 된다. 미결 재요청과 붙든 조각도 함께 살아남아
/// [`Streams::pending_resumes`] 로 다시 나간다.
///
/// ★크기를 상대가 정하는 유일한 컬렉션이다★ — 그래서 상한이 둘이다: 스트림 수(`max_streams`)와
/// 스트림당 붙드는 조각 수(`resume_buffer`).
#[derive(Debug)]
pub struct Streams<K, M> {
    seen: HashMap<K, Track<M>>,
    /// ★최근 쓴 순서★ — 뒤가 최근이다. 축출은 앞에서 한다(LRU).
    ///
    /// 등록 순서로 두면 **가장 오래 살아 있는 = 가장 활발한** 스트림이 첫 희생자가 된다.
    order: VecDeque<K>,
    max_streams: usize,
    resume_buffer: usize,
    resume_timeout: Duration,
    evicted: Vec<Evicted<K, M>>,
}

impl<K: Eq + Hash + Clone, M> Streams<K, M> {
    pub fn new(max_streams: usize, resume_buffer: usize, resume_timeout: Duration) -> Self {
        Self {
            seen: HashMap::new(),
            order: VecDeque::new(),
            max_streams: max_streams.max(1),
            resume_buffer: resume_buffer.max(1),
            resume_timeout,
            evicted: Vec::new(),
        }
    }

    /// 조각 하나를 본다. 판정은 [`Verdict`] 이고 부르는 쪽이 사건 발행·재요청·배달을 집행한다.
    pub fn observe(&mut self, mark: &StreamMark<K>, msg: M, now: Instant) -> Verdict<M> {
        if !self.seen.contains_key(&mark.key) {
            self.admit(mark);
            return Verdict::Deliver(msg);
        }
        self.touch(&mark.key);
        let resume_timeout = self.resume_timeout;
        let resume_buffer = self.resume_buffer;
        let track = self.seen.get_mut(&mark.key).expect("checked above");

        if track.generation != mark.generation {
            let from = track.generation;
            let (stale, _) = track.drain_held();
            track.generation = mark.generation;
            track.last_seq = mark.seq;
            return Verdict::GenerationChanged {
                from,
                to: mark.generation,
                msg,
                discarded: stale.len(),
            };
        }

        if mark.seq <= track.last_seq {
            return Verdict::Duplicate;
        }

        if mark.seq == track.last_seq + 1 {
            track.last_seq = mark.seq;
            // ★답이 왔다★ — 붙든 것은 버린다. 상대의 재생분이 그 구간을 다시 실어 오므로 유실이 아니고,
            //   여기서 내보내면 오히려 같은 조각이 두 번 배달된다.
            track.resume = None;
            return Verdict::Deliver(msg);
        }

        let Some(resume) = track.resume.as_mut() else {
            let resume_from = track.last_seq;
            let mut held = BTreeMap::new();
            held.insert(mark.seq, msg);
            track.resume = Some(Resume {
                after: resume_from,
                asked_at: now,
                held,
            });
            return Verdict::Gap {
                expected: resume_from + 1,
                got: mark.seq,
                resume_from,
            };
        };

        if resume.held.contains_key(&mark.seq) {
            return Verdict::Duplicate;
        }
        resume.held.insert(mark.seq, msg);
        let expired = now.saturating_duration_since(resume.asked_at) >= resume_timeout;
        let full = resume.held.len() >= resume_buffer;
        if !expired && !full {
            let awaiting_after = resume.after;
            return Verdict::Held { awaiting_after };
        }
        let (flushed, oldest) = track.drain_held();
        let oldest_valid = oldest.expect("붙든 것이 하나는 있다");
        track.last_seq = mark.seq;
        Verdict::Truncated {
            oldest_valid,
            flushed,
        }
    }

    /// ★도착 없이도 도는 경로★ — 시한이 지난 재요청을 접는다.
    ///
    /// 이것이 없으면 구멍 뒤에 스트림이 조용해진 경우(턴이 끝난 평범한 상황) 재요청이 **영원히**
    /// 미결로 남고 붙든 조각도 영영 안 나간다. [`Streams::next_resume_deadline`] 이 그 시각을 알려준다.
    pub fn expire_resumes(&mut self, now: Instant) -> Vec<Truncation<K, M>> {
        let timeout = self.resume_timeout;
        let due: Vec<K> = self
            .seen
            .iter()
            .filter(|(_, track)| {
                track
                    .resume
                    .as_ref()
                    .is_some_and(|r| now.saturating_duration_since(r.asked_at) >= timeout)
            })
            .map(|(key, _)| key.clone())
            .collect();
        due.into_iter()
            .filter_map(|key| {
                let track = self.seen.get_mut(&key)?;
                let generation = track.generation;
                let (flushed, oldest) = track.drain_held();
                let oldest_valid = oldest?;
                if let Some(last) = flushed.len().checked_sub(1) {
                    let _ = last;
                }
                track.last_seq = track.last_seq.max(oldest_valid);
                // 붙든 것의 마지막 번호까지 진도를 민다.
                track.last_seq = track.last_seq.max(oldest_valid + flushed.len() as u64 - 1);
                Some(Truncation {
                    key,
                    generation,
                    oldest_valid,
                    flushed,
                })
            })
            .collect()
    }

    /// 가장 이른 재요청 시한. `None` 이면 미결 재요청이 없다.
    pub fn next_resume_deadline(&self) -> Option<Instant> {
        self.seen
            .values()
            .filter_map(|track| {
                track
                    .resume
                    .as_ref()
                    .map(|r| r.asked_at + self.resume_timeout)
            })
            .min()
    }

    /// 자리를 만들려고 잊은 스트림들을 걷는다.
    pub fn drain_evicted(&mut self) -> Vec<Evicted<K, M>> {
        std::mem::take(&mut self.evicted)
    }

    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).expect("position 이 준 자리");
            self.order.push_back(k);
        }
    }

    fn admit(&mut self, mark: &StreamMark<K>) {
        while self.order.len() >= self.max_streams {
            let Some(victim) = self.order.pop_front() else {
                break;
            };
            if let Some(mut track) = self.seen.remove(&victim) {
                let generation = track.generation;
                let last_seq = track.last_seq;
                let (flushed, oldest_valid) = track.drain_held();
                self.evicted.push(Evicted {
                    key: victim,
                    generation,
                    last_seq,
                    flushed,
                    oldest_valid,
                });
            }
        }
        self.order.push_back(mark.key.clone());
        self.seen.insert(
            mark.key.clone(),
            Track {
                generation: mark.generation,
                last_seq: mark.seq,
                resume: None,
            },
        );
    }

    /// 아직 답을 못 받은 재요청 전부 — `(스트림, 그 번호 뒤부터)`.
    ///
    /// ★재연결 뒤 이것을 다시 내보내지 않으면 구멍이 영구히 남는다★.
    pub fn pending_resumes(&self) -> Vec<(K, u64)> {
        self.order
            .iter()
            .filter_map(|key| {
                let track = self.seen.get(key)?;
                track.resume.as_ref().map(|r| (key.clone(), r.after))
            })
            .collect()
    }

    /// 미결 재요청의 시계를 다시 감는다. 재요청을 실제로 내보낸 직후에 부른다.
    pub fn rearm_resumes(&mut self, now: Instant) {
        for track in self.seen.values_mut() {
            if let Some(resume) = track.resume.as_mut() {
                resume.asked_at = now;
            }
        }
    }

    /// 지금까지 이어받은 마지막 순번.
    pub fn last_seq(&self, key: &K) -> Option<u64> {
        self.seen.get(key).map(|t| t.last_seq)
    }

    /// 지금 알고 있는 화신 표식.
    pub fn generation(&self, key: &K) -> Option<u32> {
        self.seen.get(key).map(|t| t.generation)
    }

    /// 이 스트림을 잊는다(소비자가 구독을 끊었을 때).
    pub fn forget(&mut self, key: &K) {
        if self.seen.remove(key).is_some() {
            self.order.retain(|k| k != key);
        }
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESUME_TIMEOUT: Duration = Duration::from_secs(2);

    fn streams() -> Streams<u8, u64> {
        Streams::new(1024, 256, RESUME_TIMEOUT)
    }

    fn mark(key: u8, generation: u32, seq: u64) -> StreamMark<u8> {
        StreamMark {
            key,
            generation,
            seq,
        }
    }

    /// 조각의 payload 는 그 순번 자체로 둔다 — 배달 순서를 그대로 단언할 수 있다.
    fn see(s: &mut Streams<u8, u64>, key: u8, gen: u32, seq: u64, now: Instant) -> Verdict<u64> {
        s.observe(&mark(key, gen, seq), seq, now)
    }

    #[test]
    fn first_chunk_of_a_stream_is_accepted_whatever_its_seq() {
        let mut s = streams();
        let t0 = Instant::now();
        assert_eq!(see(&mut s, 1, 7, 900, t0), Verdict::Deliver(900));
        assert_eq!(s.last_seq(&1), Some(900));
        assert_eq!(s.generation(&1), Some(7));
    }

    #[test]
    fn contiguous_chunks_deliver() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert_eq!(see(&mut s, 1, 7, 2, t0), Verdict::Deliver(2));
        assert_eq!(see(&mut s, 1, 7, 3, t0), Verdict::Deliver(3));
        assert_eq!(s.last_seq(&1), Some(3));
    }

    #[test]
    fn already_seen_seq_is_dropped() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(see(&mut s, 1, 7, 5, t0), Verdict::Duplicate);
        assert_eq!(see(&mut s, 1, 7, 4, t0), Verdict::Duplicate);
    }

    #[test]
    fn gap_is_detected_and_asks_from_the_last_good_seq() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert_eq!(
            see(&mut s, 1, 7, 5, t0),
            Verdict::Gap {
                expected: 2,
                got: 5,
                resume_from: 1,
            }
        );
        assert_eq!(s.last_seq(&1), Some(1), "구멍 조각은 진도를 밀지 않는다");
    }

    #[test]
    fn chunks_already_in_flight_do_not_look_like_truncation() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        assert!(matches!(see(&mut s, 1, 7, 5, t0), Verdict::Gap { .. }));
        assert_eq!(
            see(&mut s, 1, 7, 6, t0 + Duration::from_millis(1)),
            Verdict::Held { awaiting_after: 1 }
        );
        assert_eq!(s.last_seq(&1), Some(1));
    }

    #[test]
    fn the_replayed_range_is_delivered_even_after_later_chunks_arrived_first() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        for seq in 2..=6 {
            assert_eq!(
                see(&mut s, 1, 7, seq, t0 + Duration::from_millis(10)),
                Verdict::Deliver(seq),
                "seq={seq}"
            );
        }
        assert_eq!(s.last_seq(&1), Some(6));
    }

    // ── B1: 붙든 것을 버리지 않고, 신고한 하한이 참이다 ──
    #[test]
    fn a_collapsed_window_delivers_what_it_held_and_names_that_first_number() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        let v = see(&mut s, 1, 7, 9, t0 + RESUME_TIMEOUT);
        assert_eq!(
            v,
            Verdict::Truncated {
                oldest_valid: 5,
                flushed: vec![5, 6, 9],
            },
            "★버린 뒤에 늦은 번호를 하한이라 신고하면 안 된다★"
        );
        assert_eq!(s.last_seq(&1), Some(9));
    }

    #[test]
    fn the_reported_lower_bound_is_a_number_we_actually_deliver() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        let Verdict::Truncated {
            oldest_valid,
            flushed,
        } = see(&mut s, 1, 7, 8, t0 + RESUME_TIMEOUT)
        else {
            panic!("창이 닫혀야 한다");
        };
        assert_eq!(flushed.first().copied(), Some(oldest_valid));
    }

    #[test]
    fn a_full_hold_buffer_collapses_the_window_without_waiting() {
        let mut s: Streams<u8, u64> = Streams::new(1024, 3, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        let v = see(&mut s, 1, 7, 7, t0);
        assert_eq!(
            v,
            Verdict::Truncated {
                oldest_valid: 5,
                flushed: vec![5, 6, 7],
            },
            "붙드는 양에 상한이 없으면 상대가 우리 메모리를 민다"
        );
    }

    // ── B2: 도착이 없어도 시한이 돈다 ──
    #[test]
    fn a_silent_peer_still_has_its_resume_window_closed() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert!(s.expire_resumes(t0 + Duration::from_millis(1)).is_empty());

        let collapsed = s.expire_resumes(t0 + RESUME_TIMEOUT);
        assert_eq!(
            collapsed,
            vec![Truncation {
                key: 1,
                generation: 7,
                oldest_valid: 5,
                flushed: vec![5],
            }],
            "★조각이 더 안 오면 아무도 이 창을 못 닫는다 — 그게 평범한 경우다★"
        );
        assert!(s.pending_resumes().is_empty());
        assert_eq!(s.last_seq(&1), Some(5));
    }

    #[test]
    fn the_resume_deadline_is_visible_so_the_loop_can_wake_on_it() {
        let mut s = streams();
        let t0 = Instant::now();
        assert_eq!(s.next_resume_deadline(), None);
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(s.next_resume_deadline(), Some(t0 + RESUME_TIMEOUT));
        s.expire_resumes(t0 + RESUME_TIMEOUT);
        assert_eq!(s.next_resume_deadline(), None);
    }

    #[test]
    fn the_earliest_of_several_resume_deadlines_wins() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 2, 7, 1, t0);
        see(&mut s, 2, 7, 9, t0 + Duration::from_millis(50));
        see(&mut s, 1, 7, 9, t0 + Duration::from_millis(100));
        assert_eq!(
            s.next_resume_deadline(),
            Some(t0 + Duration::from_millis(50) + RESUME_TIMEOUT)
        );
    }

    #[test]
    fn a_gap_after_truncation_asks_once_more() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 9, t0 + RESUME_TIMEOUT);
        assert!(matches!(
            see(&mut s, 1, 7, 20, t0 + RESUME_TIMEOUT),
            Verdict::Gap { .. }
        ));
    }

    #[test]
    fn only_one_resume_goes_out_per_gap() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        let asks = (5..12)
            .filter(|seq| matches!(see(&mut s, 1, 7, *seq, t0), Verdict::Gap { .. }))
            .count();
        assert_eq!(asks, 1);
    }

    #[test]
    fn an_outstanding_resume_survives_so_it_can_be_sent_again() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(s.pending_resumes(), vec![(1u8, 1u64)]);

        s.rearm_resumes(t0 + Duration::from_secs(60));
        assert_eq!(
            s.next_resume_deadline(),
            Some(t0 + Duration::from_secs(60) + RESUME_TIMEOUT)
        );
        assert!(s.expire_resumes(t0 + Duration::from_secs(60)).is_empty());
    }

    #[test]
    fn an_answered_resume_stops_being_pending_and_drops_what_it_held() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(see(&mut s, 1, 7, 2, t0), Verdict::Deliver(2));
        assert!(s.pending_resumes().is_empty());
        // 붙들었던 5 는 상대의 재생분이 다시 실어 온다 — 여기서 내보내면 두 번 배달된다.
        assert_eq!(see(&mut s, 1, 7, 3, t0), Verdict::Deliver(3));
    }

    #[test]
    fn generation_change_resets_the_seq_ledger_and_says_what_it_dropped() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        see(&mut s, 1, 7, 6, t0);
        assert_eq!(
            see(&mut s, 1, 8, 1, t0),
            Verdict::GenerationChanged {
                from: 7,
                to: 8,
                msg: 1,
                discarded: 2,
            }
        );
        assert!(s.pending_resumes().is_empty());
        assert_eq!(s.last_seq(&1), Some(1));
    }

    #[test]
    fn generation_is_compared_for_equality_only_never_for_order() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 9, 50, t0);
        assert!(matches!(
            see(&mut s, 1, 2, 1, t0),
            Verdict::GenerationChanged { from: 9, to: 2, .. }
        ));
    }

    #[test]
    fn streams_do_not_share_state() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 2, 7, 100, t0);
        assert_eq!(see(&mut s, 1, 7, 2, t0), Verdict::Deliver(2));
        assert_eq!(see(&mut s, 2, 7, 101, t0), Verdict::Deliver(101));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn forget_drops_one_stream_only() {
        let mut s = streams();
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 2, 7, 1, t0);
        s.forget(&1);
        assert_eq!(s.last_seq(&1), None);
        assert_eq!(s.last_seq(&2), Some(1));
    }

    // ── N5/B3: 상한과 축출 ──
    #[test]
    fn a_peer_inventing_stream_keys_cannot_grow_the_map_without_bound() {
        let mut s: Streams<u32, u64> = Streams::new(8, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        for key in 0..10_000u32 {
            s.observe(
                &StreamMark {
                    key,
                    generation: 1,
                    seq: 1,
                },
                1,
                t0,
            );
            s.drain_evicted();
        }
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn eviction_takes_the_least_recently_used_not_the_oldest_registration() {
        let mut s: Streams<u8, u64> = Streams::new(2, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 1, 1, t0);
        see(&mut s, 2, 1, 1, t0);
        // 1 을 계속 쓴다 — 가장 활발한 스트림이 첫 희생자가 되면 안 된다.
        see(&mut s, 1, 1, 2, t0);
        see(&mut s, 3, 1, 1, t0);
        assert_eq!(s.last_seq(&1), Some(2), "★가장 활발한 스트림이 살아남는다★");
        assert_eq!(s.last_seq(&2), None);
        assert_eq!(s.last_seq(&3), Some(1));
    }

    #[test]
    fn eviction_never_swallows_an_outstanding_resume_silently() {
        let mut s: Streams<u8, u64> = Streams::new(1, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 7, 1, t0);
        see(&mut s, 1, 7, 5, t0);
        assert_eq!(s.pending_resumes().len(), 1);

        see(&mut s, 2, 7, 1, t0);
        let evicted = s.drain_evicted();
        assert_eq!(
            evicted,
            vec![Evicted {
                key: 1,
                generation: 7,
                last_seq: 1,
                flushed: vec![5],
                oldest_valid: Some(5),
            }],
            "★붙들고 있던 조각이 축출과 함께 조용히 사라지면 안 된다★"
        );
    }

    #[test]
    fn evicting_a_quiet_track_reports_it_with_nothing_held() {
        let mut s: Streams<u8, u64> = Streams::new(1, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 7, 3, t0);
        see(&mut s, 2, 7, 1, t0);
        assert_eq!(
            s.drain_evicted(),
            vec![Evicted {
                key: 1,
                generation: 7,
                last_seq: 3,
                flushed: vec![],
                oldest_valid: None,
            }]
        );
    }

    #[test]
    fn forget_keeps_the_eviction_order_consistent() {
        let mut s: Streams<u8, u64> = Streams::new(2, 256, RESUME_TIMEOUT);
        let t0 = Instant::now();
        see(&mut s, 1, 1, 1, t0);
        see(&mut s, 2, 1, 1, t0);
        s.forget(&1);
        see(&mut s, 3, 1, 1, t0);
        assert_eq!(s.len(), 2);
        assert_eq!(s.last_seq(&2), Some(1));
        assert_eq!(s.last_seq(&3), Some(1));
    }
}
