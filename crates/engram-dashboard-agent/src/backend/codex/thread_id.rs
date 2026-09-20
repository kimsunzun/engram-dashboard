//! codex thread id 회수 — 우리가 켠 TUI 상태줄을 우리 PTY 바이트에서 되읽는다.
//!
//! 주어 = 이 파일. 소유하는 것 둘 = 스캐너([`ThreadIdSniffer`])와 그 값을 프로필로 넘기는 **배달선**
//! ([`observer`]). 진입점은 [`observer`] 하나이고, 스캐너는 그 안에서만 쓰인다.
//!
//! ★codex 지식이 여기 사는 이유(ADR-0216 결정 6 · ADR-0004)★: transport 는 바이트를 넘기는 제네릭
//!   자리만 갖고 무엇을 찾는지 모른다. id 모양 · 이스케이프 해체 · 예산이 전부 이 폴더 안에 산다.
//!
//! ★파일 경계 불변식 둘 — 둘 다 호출 문맥이 PTY pump 스레드이기 때문이다★(ADR-0216 결정 2·3):
//!   1. **pump 는 아무것도 기다리지 않는다.** 이 파일이 pump 위에서 하는 일은 바이트 훑기와 채널
//!      송신뿐이고, 락도 디스크 I/O 도 없다. 프로필 쓰기는 [`observer`] 가 세운 별도 스레드가 한다.
//!   2. **pump 위에서 panic 하지 않는다.** pump 의 `catch_unwind` 는 panic 을 `finish(Error)` 로
//!      옮기므로, 여기서 터지면 멀쩡히 돌던 codex 세션이 `Failed` 로 보고되고 출력이 끊긴다. 그래서
//!      범위를 벗어날 수 있는 인덱싱·슬라이싱을 쓰지 않고 `unwrap`/`expect` 도 두지 않는다.
// ADR-0216

use std::sync::mpsc;

use uuid::Uuid;

use crate::backend::SessionIdSink;
use crate::transport::PreInputByteObserver;

/// 8-4-4-12 소문자 hex.
const UUID_LEN: usize = 36;
const HYPHEN_OFFSETS: [usize; 4] = [8, 13, 18, 23];

/// 청크 경계를 넘겨 들고 가는 해체 스트림 꼬리의 길이.
///
/// ★[`UUID_LEN`] 인 것이 계약이다 — 두 갈래를 **함께** 덮어야 한다★: ① 36 자 중 35 자까지만 도착한
///   절단 후보(펌프 버퍼가 고정 4096 바이트이고 이어 붙이는 코드가 그 루프에 없다 — ADR-0216 결정 4)
///   ② 36 자가 다 왔는데 **오른쪽 이웃이 아직 없어** 판정을 미룬 후보. ②가 있어서 35 로는 모자란다.
const CARRY_BYTES: usize = UUID_LEN;

/// 이만큼 읽고도 못 찾으면 무장을 푼다.
///
/// ★비용 가드가 **아니라 오탐 가드다**★ — 상태줄은 스폰 직후 한 번 찍히고 주기 재출력이 없다(실측
///   0.155.1). 그 뒤로 이 스트림에 흐를 수 있는 것은 사용자·모델 내용이고, 거기엔 uuid 모양이 얼마든지
///   섞여 들어온다(사용자가 id 를 한 번 붙여넣는 것으로 충분하다). 무장을 유지하면 그 값을 주워 적어
///   이어받기 손잡이가 남의 것으로 바뀐다.
/// ★시계가 아니라 바이트로 재는 것도 그 때문이다★ — 벽시계로 재면 느린 기계에서 창의 크기가 달라져
///   같은 입력이 다른 결말을 낸다.
/// ★이것은 무장 창을 닫는 **셋째** 빗장이다★ — 앞의 둘은 회수 성공(이 파일)과 **첫 입력**(통로가
///   닫는다 — [`PreInputByteObserver`] doc)이고, 먼저 오는 것이 이긴다(ADR-0216 결정 5).
const SCAN_BUDGET_BYTES: usize = 1 << 20;

const ESC: u8 = 0x1B;
const BEL: u8 = 0x07;
/// 진행 중인 이스케이프 열을 **중도 포기**시키는 두 바이트(ECMA-48 의 CAN·SUB).
///
/// ★이 둘을 안 보면 망가진 열 하나가 스트림을 통째로 삼킨다★ — 닫히지 않는 열은 뒤따르는 본문을 전부
///   열의 몸통으로 먹어, 상태줄이 그 안에 들어가면 예산이 다 떨어질 때까지 한 글자도 안 나온다.
const CAN: u8 = 0x18;
const SUB: u8 = 0x1A;

/// 해체한 이스케이프 열 자리에 **대신 남기는** 한 바이트.
///
/// ★지우지 않고 자리를 남기는 것이 결정이다★ — 이스케이프 열은 글자를 잇는 풀이 아니라 **경계**다.
///   ① 통째로 지우면 화면의 서로 다른 두 조각이 맞붙어 없던 uuid 모양을 만들어 낸다. ② 상태줄
///   뒤에 한동안 이스케이프만 오는 프레임에서, 지워 버리면 오른쪽 이웃이 영영 안 와 판정이 무기한
///   미뤄진다.
/// ★대가 = uuid **한가운데**를 이스케이프가 가르면 못 잡는다 — 미검이다★. ratatui 계열은 같은 스타일
///   구간을 한 번에 쓰므로 첫 프레임의 id 가 갈릴 이유는 없지만, 그것을 실측한 적은 없다. 못 잡은
///   결말은 기존 동작 그대로다(ADR-0216 결정 8).
const ESCAPE_MARK: u8 = b'\n';

/// ANSI 이스케이프 해체 상태 — ★청크를 넘겨 유지된다★(한 열이 두 `read` 에 쪼개진다).
///
/// ★모든 갈래가 **유계**다 — VT 파서를 들이지 않는 것이 결정이다★: 여기서 필요한 것은 「어디까지가
///   열이고 어디부터가 화면 글자인가」 하나뿐이고, 파라미터의 뜻은 한 번도 안 읽는다.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ansi {
    Ground,
    /// `ESC` 를 봤다. 다음 바이트가 무슨 열을 여는지 아직 모른다.
    Escape,
    /// `ESC` 뒤 중간 바이트(`0x20..=0x2F`)를 봤다 — 최종 바이트가 올 때까지.
    ///
    /// ★이 상태가 없으면 **진짜 id 를 거절한다**★: `ESC ( B`(G0 을 ASCII 로 지정 — ConPTY·ratatui 가
    ///   흔히 낸다)를 두 바이트 열로 읽으면 `B` 가 화면 글자로 남고, 바로 뒤에 붙은 id 의 **왼쪽 이웃이
    ///   hex 글자**가 되어 경계 판정이 그 id 를 잘린 도막으로 본다.
    EscapeIntermediate,
    /// `ESC [` — 최종 바이트(`0x40..=0x7E`)까지.
    Csi,
    /// 문자열 몸통을 끄는 열 — `ESC ]`(OSC) · `ESC P`(DCS) · `ESC X`(SOS) · `ESC ^`(PM) ·
    /// `ESC _`(APC). `BEL` 이나 ST(`ESC \`)까지.
    ///
    /// ★DCS·SOS·PM·APC 를 두 바이트 열로 읽으면 **그 몸통이 화면 글자로 샌다**★ — 그 안에 hex 열이
    ///   있으면 없던 후보가 생기고, 우리 id 옆에 붙으면 경계 판정을 어긋나게 한다.
    /// ★**`BEL` 은 다섯 모두를 닫는다 — 규격대로라면 OSC 만 그렇다**★. 알고 고른 단순화다: 실 TUI 가
    ///   내는 것은 CSI 와 OSC 뿐이고(ratatui·ConPTY), 다섯을 갈라 적으면 상태가 둘로 늘면서 얻는 것이
    ///   없다. ★남는 잔여 = DCS·SOS·PM·APC 몸통에 `0x07` 이 섞여 오면 거기서 일찍 닫히고 나머지 몸통이
    ///   화면 글자로 샌다★ — 없던 후보가 생길 수 있는 갈래이고, 그 폭발 반경은 예산([`SCAN_BUDGET_BYTES`])
    ///   과 CAN·SUB 포기가 묶는다.
    StringBody,
    /// 문자열 몸통 안에서 `ESC` 를 봤다 — 다음이 `\` 면 열이 닫힌다.
    StringBodyEscape,
}

/// 터미널 바이트에서 codex thread id 를 한 번 건져 내는 스캐너.
///
/// 성공은 **많아야 한 번**이다 — 처음 잡은 뒤 스스로 무장을 풀고 그 뒤 [`ThreadIdSniffer::feed`] 는
/// 언제나 `None` 을 돌려준다. 예산([`SCAN_BUDGET_BYTES`])을 다 써도 같다.
struct ThreadIdSniffer {
    ansi: Ansi,
    /// 해체된 스트림의 꼬리. 청크 경계를 넘는 후보를 여기서 잇는다.
    pending: Vec<u8>,
    /// `pending[0]` **바로 앞** 바이트가 id 글자였나 — 잘라 버린 왼쪽 이웃을 대신 기억한다.
    /// ★없으면 경계 판정이 한 칸 비어, 잘린 긴 hex 열의 뒷도막이 온전한 id 로 읽힌다.★
    carry_left_adjacent: bool,
    /// 남은 스캔 예산. `0` = 소진.
    budget: usize,
    armed: bool,
}

impl ThreadIdSniffer {
    fn new() -> Self {
        Self {
            ansi: Ansi::Ground,
            pending: Vec::new(),
            carry_left_adjacent: false,
            budget: SCAN_BUDGET_BYTES,
            armed: true,
        }
    }

    /// 갓 읽은 바이트 슬라이스를 밀어 넣고, **이번 호출에서 확정된** id 만 돌려준다.
    ///
    /// `None` 의 뜻은 셋 중 하나다 — 아직 못 찾았다 · 후보는 섰는데 오른쪽 이웃이 안 와 판정을
    /// 미뤘다 · 이미 무장이 풀렸다. 호출자가 그 셋을 가를 필요는 없다(셋 다 「지금은 적을 것이 없다」).
    fn feed(&mut self, chunk: &[u8]) -> Option<Uuid> {
        if !self.armed {
            return None;
        }
        // ★예산은 **들여보내는 자리**에서 끊는다 — 다 훑고 나서 재면 늦다★: 한도를 넘어 도착한 바이트도
        //   일단 버퍼에 들어가면 그 안에서 선 후보가 그대로 채택돼, 「한도 밖에서 시작한 매치」가 통과한다
        //   (예산이 오탐 가드라는 성질이 거기서 무너진다). 한도까지만 잘라 넣으면 그 후보는 **형성 자체가
        //   안 된다**.
        // ★대가 = 한도 경계에 정확히 걸친 id 는 잃는다★ — 미검이고, 기존 동작 그대로다(ADR-0216 결정 8).
        //   반대 방향(한도 밖 값을 주워 적는 것)은 이어받기 손잡이를 남의 값으로 바꾼다.
        // ★닿을 수 없는 갈래를 **fail-closed 로** 적는다★ — `take` 는 위에서 `chunk.len()` 으로 눌러
        //   놨으니 `get` 이 `None` 을 낼 수 없지만, 그 산술이 언젠가 바뀌면 `unwrap_or(chunk)` 는 방금
        //   계산한 한도를 무시하고 **청크 전체**를 들여보낸다 — 이 절이 막으려던 바로 그 구멍이다.
        let take = self.budget.min(chunk.len());
        self.budget = self.budget.saturating_sub(take);
        self.strip_into_pending(chunk.get(..take).unwrap_or(&[]));
        let found = self.scan();
        if found.is_some() || self.budget == 0 {
            self.armed = false;
            self.pending = Vec::new();
        } else {
            self.trim();
        }
        found
    }

    fn strip_into_pending(&mut self, chunk: &[u8]) {
        for &b in chunk {
            // ★CAN·SUB 는 어느 열 안에서든 즉시 포기다 — 상태마다 되풀어 적지 않고 여기서 한 번 가른다★
            //   (Ground 에서는 평범한 제어문자라 그냥 실린다 — id 글자가 아니므로 경계로 선다).
            if (b == CAN || b == SUB) && self.ansi != Ansi::Ground {
                self.close_sequence();
                continue;
            }
            match self.ansi {
                Ansi::Ground => {
                    if b == ESC {
                        self.ansi = Ansi::Escape;
                    } else {
                        self.pending.push(b);
                    }
                }
                Ansi::Escape => match b {
                    b'[' => self.ansi = Ansi::Csi,
                    // 문자열 몸통을 끄는 넷 — OSC · DCS · SOS · PM · APC.
                    b']' | b'P' | b'X' | b'^' | b'_' => self.ansi = Ansi::StringBody,
                    // 중간 바이트가 붙는 열(`ESC ( B` 등) — 최종 바이트는 아직 안 왔다.
                    0x20..=0x2F => self.ansi = Ansi::EscapeIntermediate,
                    // 이어진 `ESC` 는 앞 열을 버리고 새 열을 연다.
                    ESC => {}
                    // 나머지는 두 바이트짜리 열이라 여기서 닫힌다.
                    _ => self.close_sequence(),
                },
                Ansi::EscapeIntermediate => match b {
                    0x20..=0x2F => {}
                    ESC => self.ansi = Ansi::Escape,
                    _ => self.close_sequence(),
                },
                Ansi::Csi => match b {
                    // 파라미터·중간 바이트는 그냥 지난다 — 뜻을 읽지 않는다.
                    ESC => self.ansi = Ansi::Escape,
                    0x40..=0x7E => self.close_sequence(),
                    _ => {}
                },
                Ansi::StringBody => match b {
                    BEL => self.close_sequence(),
                    ESC => self.ansi = Ansi::StringBodyEscape,
                    _ => {}
                },
                Ansi::StringBodyEscape => match b {
                    b'\\' => self.close_sequence(),
                    // ★`BEL` 은 여기서도 닫는다 — 이 갈래를 빼면 몸통이 안 닫혀 뒤따르는 상태줄을
                    //   통째로 삼킨다★(`ESC ] 0 ; x ESC BEL` 처럼 몸통 끝에 `ESC` 가 붙어 오는 모양).
                    //   `StringBody` 의 같은 갈래와 **짝이다 — 한쪽만 두지 말 것**.
                    BEL => self.close_sequence(),
                    // 이어진 `ESC` — 아직 ST 일 수 있다.
                    ESC => {}
                    // ST 가 아니었다 — 몸통은 아직 열려 있다.
                    _ => self.ansi = Ansi::StringBody,
                },
            }
        }
    }

    /// 열이 닫혔다(또는 포기됐다) — 그 자리에 경계 한 바이트를 남기고 화면 글자로 돌아간다.
    fn close_sequence(&mut self) {
        self.ansi = Ansi::Ground;
        self.pending.push(ESCAPE_MARK);
    }

    fn scan(&self) -> Option<Uuid> {
        let buf = &self.pending;
        let mut i = 0usize;
        // ★인덱싱 대신 `get` 으로만 창을 뜬다★ — 범위를 벗어나면 panic 이 아니라 `None` 이라야 한다
        //   (파일 헤더 불변식 2).
        while let Some(win) = buf.get(i..i.saturating_add(UUID_LEN)) {
            if !is_uuid_shaped(win) {
                i = i.saturating_add(1);
                continue;
            }
            let left_adjacent = match i.checked_sub(1) {
                Some(prev) => buf.get(prev).copied().is_some_and(is_adjacent),
                None => self.carry_left_adjacent,
            };
            if left_adjacent {
                i = i.saturating_add(1);
                continue;
            }
            match buf.get(i.saturating_add(UUID_LEN)) {
                // 오른쪽 이웃이 아직 안 왔다 — 더 긴 열의 앞도막일 수 있으므로 판정을 미룬다.
                //   이 후보는 `CARRY_BYTES` 가 그대로 다음 청크로 넘긴다.
                None => return None,
                Some(&next) if is_adjacent(next) => {
                    i = i.saturating_add(1);
                    continue;
                }
                Some(_) => {}
            }
            // 모양이 선 창은 ASCII 이고 uuid 문법을 만족하므로 이 둘은 실패할 수 없다. 그래도 물어서
            //   받는다 — 실패를 panic 으로 바꾸지 않는 것이 이 파일의 불변식이다.
            if let Ok(text) = std::str::from_utf8(win) {
                if let Ok(id) = Uuid::parse_str(text) {
                    return Some(id);
                }
            }
            i = i.saturating_add(1);
        }
        None
    }

    fn trim(&mut self) {
        let Some(drop_at) = self.pending.len().checked_sub(CARRY_BYTES) else {
            return;
        };
        let Some(prev) = drop_at.checked_sub(1) else {
            return;
        };
        self.carry_left_adjacent = self.pending.get(prev).copied().is_some_and(is_adjacent);
        self.pending = self.pending.iter().skip(drop_at).copied().collect();
    }
}

/// 8-4-4-12 소문자 hex 인가.
///
/// ★대문자를 받지 않는다★ — codex 가 내는 것은 소문자이고(실측 0.155.1), 대소문자를 함께 받으면
///   매칭 대상이 넓어지는 만큼 화면 텍스트에서 우연히 걸릴 창도 넓어진다.
fn is_uuid_shaped(win: &[u8]) -> bool {
    win.iter().enumerate().all(|(k, &b)| {
        if HYPHEN_OFFSETS.contains(&k) {
            b == b'-'
        } else {
            matches!(b, b'0'..=b'9' | b'a'..=b'f')
        }
    })
}

/// 이 바이트가 후보에 **붙어 있으면 그 후보는 더 긴 열의 도막**이라는 뜻인가.
///
/// ★여기서는 대문자 hex 도 센다 — 위 [`is_uuid_shaped`] 와 갈리는 것이 의도다★: 매칭은 좁히고 경계는
///   넓혀야 「긴 열을 잘라 id 라 부르는」 갈래가 막힌다.
fn is_adjacent(b: u8) -> bool {
    b.is_ascii_hexdigit() || b == b'-'
}

/// 스캐너를 pump 에 꽂을 관찰자로 감싸고, 건진 id 를 적을 **한 칸짜리 배달선**을 함께 세운다.
///
/// ★pump 가 붙드는 것은 송신자 하나뿐이다 — 그것이 이 함수가 있는 이유다(ADR-0216 결정 2)★:
///   `sink` 가 닿는 프로필 레지스트리는 전역 뮤텍스를 쥔 채 명부 전체를 디스크에 저장하고(ADR-0207),
///   그 락은 `expect` 로 풀린다. pump 에서 직접 부르면 두 가지가 한꺼번에 깨진다 — ① 유일한 PTY
///   리더가 전역 락과 디스크 I/O 뒤에 서서 자식 프로세스에 역압이 걸리고 ② pump 의 panic 이
///   **다른 에이전트까지 닿는** 락을 오염시킨다(pump 의 unwind-safety 정당화가 「이 스레드가 붙드는
///   것은 그 에이전트 전용뿐」에 기대 서 있다).
/// ★스레드 수명 — join 하는 자리가 없는 것이 계약이다★: 첫 값을 적으면 끝나고, 값 없이 pump 가
///   끝나면 송신자가 떨어져 `recv` 가 `Err` 로 깨어나 끝난다. 어느 쪽이든 스스로 거둬진다.
/// ★스레드를 못 띄워도 스폰은 간다★ — 경고만 남기고 관찰자는 그대로 돌려준다(보낸 값이 갈 곳이 없을
///   뿐이다). ADR-0216 결정 8.
// ADR-0216
pub(crate) fn observer(sink: SessionIdSink) -> PreInputByteObserver {
    let (tx, rx) = mpsc::channel::<Uuid>();
    if let Err(e) = std::thread::Builder::new()
        .name("engram-codex-thread-id".into())
        .spawn(move || {
            if let Ok(id) = rx.recv() {
                sink(&id.to_string());
            }
        })
    {
        tracing::warn!("codex thread id 기록 스레드를 못 띄웠다 — 이 화신은 id 를 못 적는다: {e}");
    }
    let mut sniffer = ThreadIdSniffer::new();
    Box::new(move |bytes: &[u8]| {
        if let Some(id) = sniffer.feed(bytes) {
            // 받는 쪽이 떠났으면(스레드를 못 띄웠거나 이미 적고 끝났다) 적을 곳이 없다 — pump 가 할 수
            //   있는 일도 알아야 할 일도 없다.
            let _ = tx.send(id);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const ID: &str = "019a0b1c-2d3e-4f50-8a9b-c0d1e2f30405";

    fn id() -> Uuid {
        Uuid::parse_str(ID).expect("고정 상수")
    }

    fn sniff_all(chunks: &[&[u8]]) -> Option<Uuid> {
        let mut s = ThreadIdSniffer::new();
        let mut found = None;
        for c in chunks {
            if let Some(got) = s.feed(c) {
                found = Some(got);
            }
        }
        found
    }

    // ── 잡는다 ────────────────────────────────────────────────────────────────

    #[test]
    fn a_plain_status_line_yields_the_id() {
        let line = format!("  thread-id: {ID}  \r\n");
        assert_eq!(sniff_all(&[line.as_bytes()]), Some(id()));
    }

    /// 실물에 가장 가까운 모양 — 색·커서 이동이 값을 감싼다.
    #[test]
    fn ansi_wrapped_output_yields_the_id() {
        let line = format!("\x1b[2K\x1b[38;5;244mthread-id: \x1b[0m{ID}\x1b[0m\x1b[1;1H");
        assert_eq!(sniff_all(&[line.as_bytes()]), Some(id()));
    }

    /// ★펌프 버퍼가 고정 4096 이고 이어 붙이는 코드가 그 루프에 없다 — 이 항목이 그 carry 를 잰다★
    /// (ADR-0216 결정 4). 모든 절단 자리에서 갈라 본다.
    #[test]
    fn an_id_split_across_reads_is_reassembled() {
        let line = format!("id={ID} ");
        for cut in 1..line.len() {
            let (a, b) = line.as_bytes().split_at(cut);
            assert_eq!(
                sniff_all(&[a, b]),
                Some(id()),
                "{cut} 번째 바이트에서 갈랐을 때 못 이었다"
            );
        }
    }

    /// 한 이스케이프 열이 두 `read` 에 쪼개져도 해체 상태가 이어져야 한다.
    #[test]
    fn an_escape_sequence_split_across_reads_is_still_stripped() {
        let tail = format!("38;5;244mthread-id: {ID} ");
        assert_eq!(sniff_all(&[b"\x1b[", tail.as_bytes()]), Some(id()));
    }

    /// 문자열 몸통을 끄는 열 다섯은 `BEL` 로도 ST(`ESC \`)로도 닫힌다 — 어느 쪽이든 그 뒤 본문을 다시
    /// 읽어야 하고, ★몸통 안의 글자가 화면 글자로 새면 안 된다★.
    #[test]
    fn a_string_sequence_does_not_swallow_the_rest_of_the_stream() {
        for opener in [']', 'P', 'X', '^', '_'] {
            for terminator in ["\x07", "\x1b\\"] {
                // 몸통 안에 hex 열을 넣어 둔다 — 새면 우리 id 의 왼쪽 이웃이 되어 판정이 어긋난다.
                let line = format!("\x1b{opener}0;deadbeef{terminator}{ID} ");
                assert_eq!(
                    sniff_all(&[line.as_bytes()]),
                    Some(id()),
                    "`ESC {opener}` 열({terminator:?} 종단)의 몸통이 샜거나 본문을 못 읽었다"
                );
            }
        }
    }

    /// ★몸통 끝에 `ESC` 가 붙어 온 뒤의 `BEL` 도 몸통을 닫는다★ — `ESC` 를 보고 ST 를 기다리는 상태에서
    /// `\` 만 종단으로 치면 그 `BEL` 이 몸통 글자로 먹히고, 열이 안 닫힌 채 뒤따르는 상태줄을 통째로
    /// 삼킨다. `StringBody` 와 `StringBodyEscape` 의 `BEL` 갈래는 짝이다.
    #[test]
    fn a_bel_after_an_escape_still_closes_the_string_body() {
        for opener in [']', 'P'] {
            let line = format!("\x1b{opener}0;x\x1b\x07{ID} ");
            assert_eq!(
                sniff_all(&[line.as_bytes()]),
                Some(id()),
                "`ESC {opener}` 몸통이 `ESC BEL` 로 안 닫혀 뒤 본문을 삼켰다"
            );
        }
    }

    /// ★중간 바이트가 붙는 열을 두 바이트로 읽으면 **진짜 id 를 거절한다**★ — `ESC ( B` 의 `B` 가 화면
    /// 글자로 남아 바로 뒤 id 의 왼쪽 이웃이 hex 가 된다(ConPTY·ratatui 가 흔히 내는 모양이다).
    #[test]
    fn an_escape_with_intermediate_bytes_does_not_leave_its_final_byte_behind() {
        assert_eq!(sniff_all(&[format!("\x1b(B{ID} ").as_bytes()]), Some(id()));
    }

    /// ★CAN·SUB 는 어느 열 안에서든 즉시 포기다★ — 안 그러면 닫히지 않는 열 하나가 뒤따르는 상태줄을
    /// 통째로 삼켜 예산이 떨어질 때까지 한 글자도 안 나온다.
    #[test]
    fn a_truncated_sequence_is_aborted_instead_of_swallowing_the_stream() {
        for abort in ['\x18', '\x1a'] {
            for opener in ["[38;5;", "]0;codex", "P1;2"] {
                let line = format!("\x1b{opener}{abort}{ID} ");
                assert_eq!(
                    sniff_all(&[line.as_bytes()]),
                    Some(id()),
                    "`ESC {opener}` 가 {abort:?} 로 안 끊겨 뒤 본문을 삼켰다"
                );
            }
        }
    }

    /// 오른쪽 이웃이 아직 없으면 미뤘다가, 다음 청크가 경계를 확정하면 그때 잡는다.
    #[test]
    fn a_candidate_at_the_end_of_a_chunk_is_decided_on_the_next_one() {
        let mut s = ThreadIdSniffer::new();
        assert_eq!(s.feed(format!("thread-id: {ID}").as_bytes()), None);
        assert_eq!(s.feed(b"\x1b[0m"), Some(id()));
    }

    // ── 안 잡는다 ─────────────────────────────────────────────────────────────

    /// ★긴 hex 열을 잘라 id 라 부르지 않는다★ — 양쪽 이웃을 다 본다.
    #[test]
    fn a_longer_hex_run_is_not_clipped_into_an_id() {
        for line in [
            format!("f{ID} "),
            format!(" {ID}f"),
            format!("-{ID} "),
            format!(" {ID}F"),
        ] {
            assert_eq!(sniff_all(&[line.as_bytes()]), None, "{line} 을 잘라 잡았다");
        }
    }

    /// 같은 판정이 **청크 경계를 넘어서도** 서야 한다 — 후보가 첫 청크 끝에 걸려 판정이 미뤄지면
    /// 그 왼쪽 이웃은 carry 에서 잘려 나간 뒤다.
    #[test]
    fn the_left_boundary_survives_the_carry_over() {
        let mut s = ThreadIdSniffer::new();
        assert_eq!(s.feed(format!("abcdf{ID}").as_bytes()), None);
        assert_eq!(
            s.feed(b" "),
            None,
            "잘려 나간 왼쪽 이웃을 못 기억해 긴 열의 뒷도막을 잡았다"
        );
    }

    /// ★이스케이프 자리에 경계를 남기는 이유★ — 통째로 지우면 이 둘이 맞붙어 없던 id 가 생긴다.
    #[test]
    fn two_screen_fragments_are_not_glued_into_an_id() {
        let (left, right) = ID.split_at(20);
        let line = format!("{left}\x1b[7;1H{right} ");
        assert_eq!(sniff_all(&[line.as_bytes()]), None);
    }

    #[test]
    fn an_uppercase_uuid_is_not_matched() {
        let line = format!("thread-id: {} ", ID.to_ascii_uppercase());
        assert_eq!(sniff_all(&[line.as_bytes()]), None);
    }

    /// 자리가 어긋난 하이픈은 uuid 가 아니다.
    #[test]
    fn a_misplaced_hyphen_is_not_matched() {
        let line = "0123456-789abcdef-0123-4567-89abcdef0123 ";
        assert_eq!(sniff_all(&[line.as_bytes()]), None);
    }

    // ── 무장 해제 ─────────────────────────────────────────────────────────────

    /// 첫 성공 뒤로는 아무것도 안 돌려준다(ADR-0216 결정 5 ⓐ).
    #[test]
    fn the_sniffer_disarms_after_the_first_hit() {
        let mut s = ThreadIdSniffer::new();
        assert!(s.feed(format!("{ID} ").as_bytes()).is_some());
        let other = "0198f0e1-d2c3-4b5a-8697-1a2b3c4d5e6f";
        assert_eq!(s.feed(format!("{other} ").as_bytes()), None);
    }

    /// 예산을 다 쓰면 못 찾은 채로 무장을 푼다 — 그 뒤의 uuid 모양은 우리 것이 아니다
    /// (ADR-0216 결정 5 ⓒ).
    #[test]
    fn the_sniffer_disarms_when_the_budget_runs_out() {
        let mut s = ThreadIdSniffer::new();
        let filler = vec![b'.'; 4096];
        let mut fed = 0usize;
        while fed <= SCAN_BUDGET_BYTES {
            assert_eq!(s.feed(&filler), None);
            fed += filler.len();
        }
        assert_eq!(s.feed(format!(" {ID} ").as_bytes()), None);
    }

    /// ★한도를 **넘어서 시작하는** 매치는 같은 청크 안에 있어도 안 된다★ — 다 훑고 나서 예산을 재던
    /// 옛 형태는 마지막 한 바이트를 남겨 두고 들어온 청크의 id 를 그대로 주웠다.
    #[test]
    fn a_match_that_begins_past_the_budget_is_not_taken() {
        let mut s = ThreadIdSniffer::new();
        let mut fed = 0usize;
        let filler = vec![b'.'; 4096];
        while fed + filler.len() < SCAN_BUDGET_BYTES {
            assert_eq!(s.feed(&filler), None);
            fed += filler.len();
        }
        // 한도가 정확히 1 바이트 남게 맞춘다.
        let gap = SCAN_BUDGET_BYTES - fed - 1;
        assert_eq!(s.feed(&vec![b'.'; gap]), None);
        assert_eq!(
            s.feed(format!(".{ID} ").as_bytes()),
            None,
            "한도 밖에서 시작한 매치를 주웠다"
        );
    }

    // ── 배달선 ────────────────────────────────────────────────────────────────

    /// ★관찰자 클로저는 `sink` 를 붙들지 않는다 — 그 호출은 별도 스레드에서 일어난다★.
    /// 여기서 재는 것은 그 경계를 건너 값이 실제로 도착한다는 것 하나다.
    #[test]
    fn the_observer_hands_the_id_off_to_the_sink() {
        let (tx, rx) = mpsc::channel::<String>();
        let tx = Mutex::new(tx);
        let sink: SessionIdSink = Arc::new(move |raw: &str| {
            if let Ok(g) = tx.lock() {
                let _ = g.send(raw.to_string());
            }
        });
        let mut obs = observer(sink);
        obs(format!("thread-id: {ID} ").as_bytes());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ID.to_string())
        );
    }
}
