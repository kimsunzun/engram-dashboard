//! The words of the approval questions, in each UI language the app has (English source, Korean translation — ADR-0372). Every
//!   question is built in both languages when it is asked; the seam picks one when the dialog shows.
//!
//! ★What a string from outside looks like here★ ([`visible`], [`quote`]): an agent or a file can put any character in a name, an
//!   address or a URL, so every such string is shown on one line with `"` and `\` escaped and every whitespace other than a plain space,
//!   every control character and every invisible format character (bidi overrides, zero-width — `trusted::is_invisible`) replaced by a
//!   visible U+FFFD — the system-written labels around it cannot be imitated or reordered. The system-written agent id comes before the
//!   agent's own name. A helper URL shows its host and port in full (ASCII by its parse rule) and caps only the path and query, so a long
//!   host cannot hide its real domain. The other PC's ID is always the full grouped ID, never a short fingerprint.

// ADR-0394, ADR-0372

use std::net::SocketAddr;

use super::super::id::PeerId;
use super::super::trusted::{is_invisible, Helper, NetworkSettings, TrustedPeer};
use unicode_segmentation::UnicodeSegmentation;

use super::super::trusted::NetworkPatch;
use crate::ask_user::{Lang, Question, QuestionText};

/// Characters of an outside string shown (more is cut with `…`).
pub(crate) const QUOTE_CAP: usize = 80;
/// Characters of a URL's path and query shown.
const PATH_CAP: usize = 40;
/// Characters of an agent id shown.
const AGENT_ID_PREFIX: usize = 8;
/// Stacking combining marks kept per user-perceived character.
pub(crate) const MAX_MARKS: usize = 2;
/// Code points kept per user-perceived character — marks outside the stacking blocks (Thai, Arabic, Hebrew …) are bounded by this; a
///   syllable in conjoining jamo, an Indic conjunct and most emoji ZWJ sequences fit (a long one — e.g. a family with skin tones — is cut).
pub(crate) const MAX_CLUSTER_CHARS: usize = 8;

/// A combining mark from the blocks whose marks stack above or below a letter (Combining Diacritical Marks and its Extended and
///   Supplement blocks, Combining Marks for Symbols — the enclosing marks among them — and Combining Half Marks). Script-specific marks
///   (Indic vowel signs, viramas) are not counted: a syllable needs them, and they do not stack without end.
fn is_stacking_mark(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}
/// Trusted PCs listed in the accept-list question; the rest are counted.
pub(crate) const LISTED_PCS: usize = 5;

const LANGS: [Lang; 2] = [Lang::En, Lang::Ko];

// The Korean text of this table sits in the block below (the Hangul check allows it here only).
// hangul-ok-begin

/// `text` made safe to show: one line, `"` (and the quote look-alikes) and `\` escaped, every other whitespace, control or invisible
///   character a visible U+FFFD, each user-perceived character keeping at most [`MAX_MARKS`] stacking combining marks and at most
///   [`MAX_CLUSTER_CHARS`] code points in all (what is dropped shows as one U+FFFD per character — a stack of marks cannot draw over
///   the line above or below), cut at `cap` such characters.
pub(crate) fn visible(text: &str, cap: usize) -> String {
    let mut out = String::new();
    let mut clusters = text.graphemes(true);
    for cluster in clusters.by_ref().take(cap) {
        let mut marks = 0;
        let mut kept = 0;
        let mut dropped = false;
        for c in cluster.chars() {
            if is_stacking_mark(c) {
                marks += 1;
                if marks > MAX_MARKS {
                    dropped = true;
                    continue;
                }
            }
            if kept == MAX_CLUSTER_CHARS {
                dropped = true;
                continue;
            }
            kept += 1;
            match c {
                '"' | '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}' | '\u{201E}'
                | '\u{201F}' | '\u{00AB}' | '\u{00BB}' | '\u{2033}' | '\u{2036}' | '\u{275D}'
                | '\u{275E}' | '\u{301D}' | '\u{301E}' | '\u{301F}' | '\u{FF02}' | '\u{02BA}'
                | '\u{3003}' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                ' ' => out.push(' '),
                c if c.is_whitespace() || c.is_control() || is_invisible(c) => out.push('\u{FFFD}'),
                c => out.push(c),
            }
        }
        if dropped {
            out.push('\u{FFFD}');
        }
    }
    if clusters.next().is_some() {
        out.push('…');
    }
    out
}

/// An outside string in quotes ([`visible`], capped at [`QUOTE_CAP`]).
pub(crate) fn quote(text: &str) -> String {
    format!("\"{}\"", visible(text, QUOTE_CAP))
}

/// A helper URL: `scheme://host:port` in full, then the path and query quoted and capped.
pub(crate) fn url_text(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some(split) => split,
        None => return quote(url),
    };
    let cut = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(cut);
    let head = format!(
        "{}://{}",
        visible(scheme, QUOTE_CAP),
        visible(authority, usize::MAX)
    );
    if tail.is_empty() || tail == "/" {
        head
    } else {
        format!("{head} {}", format!("\"{}\"", visible(tail, PATH_CAP)))
    }
}

fn pick(lang: Lang, en: &str, ko: &str) -> String {
    match lang {
        Lang::En => en.to_string(),
        Lang::Ko => ko.to_string(),
    }
}

fn helper_text(lang: Lang, helper: &Helper) -> String {
    match helper {
        Helper::Default => pick(
            lang,
            "n0's public servers (default)",
            "n0 공용 서버(기본값)",
        ),
        Helper::None => pick(lang, "none", "없음"),
        Helper::Url(url) => url_text(url),
    }
}

fn port_text(lang: Lang, port: Option<u16>) -> String {
    match port {
        Some(port) => port.to_string(),
        None => pick(lang, "default", "기본값"), // hangul-ok
    }
}

fn on_off(lang: Lang, on: bool) -> String {
    match (lang, on) {
        (Lang::En, true) => "on".to_string(),
        (Lang::En, false) => "off".to_string(),
        (Lang::Ko, true) => "켬".to_string(),  // hangul-ok
        (Lang::Ko, false) => "끔".to_string(), // hangul-ok
    }
}

fn address_text(lang: Lang, address: Option<SocketAddr>) -> String {
    match address {
        Some(address) => quote(&address.to_string()),
        None => pick(lang, "none", "없음"), // hangul-ok
    }
}

struct Labels {
    name: &'static str,
    id: &'static str,
    address: &'static str,
    port: &'static str,
    relay: &'static str,
    lookup: &'static str,
    mdns: &'static str,
}

const LABELS_EN: Labels = Labels {
    name: "PC name",
    id: "PC ID",
    address: "Direct address",
    port: "Listen port",
    relay: "Relay server",
    lookup: "Address lookup",
    mdns: "Local network discovery (mDNS)",
};

const LABELS_KO: Labels = Labels {
    name: "PC 이름",
    id: "PC ID",
    address: "직접 주소",
    port: "수신 포트",
    relay: "릴레이 서버",
    lookup: "주소 조회",
    mdns: "로컬 네트워크 탐색(mDNS)",
};

fn labels(lang: Lang) -> &'static Labels {
    match lang {
        Lang::En => &LABELS_EN,
        Lang::Ko => &LABELS_KO,
    }
}

fn peer_lines(lang: Lang, peer: &TrustedPeer) -> String {
    let l = labels(lang);
    format!(
        "{}: {}\n{}: {}\n{}: {}",
        l.name,
        quote(&peer.name),
        l.id,
        peer.id.grouped(),
        l.address,
        address_text(lang, peer.address)
    )
}

fn network_lines(lang: Lang, network: &NetworkSettings) -> String {
    let l = labels(lang);
    format!(
        "{}: {}\n{}: {}\n{}: {}\n{}: {}",
        l.port,
        port_text(lang, network.port),
        l.relay,
        helper_text(lang, &network.relay),
        l.lookup,
        helper_text(lang, &network.address_lookup),
        l.mdns,
        on_off(lang, network.mdns)
    )
}

/// What the agent's change does, as the question shows it (approvals fills it from the state at request time).
pub(crate) enum Asked<'a> {
    Add {
        peer: &'a TrustedPeer,
        /// That ID knocked (tried to link while not listed) this run.
        knocked: bool,
        /// Another ID held this name earlier this run.
        name_was: Option<PeerId>,
    },
    SetNetwork {
        from: &'a NetworkSettings,
        patch: &'a NetworkPatch,
    },
    RegenerateKey {
        /// The adopt question is open — this abandons the saved identity instead.
        instead_of_adopting: bool,
    },
    Rename {
        id: PeerId,
        from_name: Option<&'a str>,
        to_name: Option<&'a str>,
        from_address: Option<SocketAddr>,
        to_address: Option<Option<SocketAddr>>,
    },
}

impl Asked<'_> {
    fn kind(&self) -> &'static str {
        match self {
            Asked::Add { .. } => "add",
            Asked::SetNetwork { .. } => "setNetwork",
            Asked::RegenerateKey { .. } => "regenerateKey",
            Asked::Rename { .. } => "rename",
        }
    }
}

/// The agent behind a request: the system-written id first, then its own name.
fn agent_line(
    lang: Lang,
    agent_id: &str,
    agent_name: Option<&str>,
    origin: Option<&str>,
) -> String {
    let prefix = visible(
        &agent_id.chars().take(AGENT_ID_PREFIX).collect::<String>(),
        AGENT_ID_PREFIX,
    );
    let mut line = match (lang, agent_name) {
        (Lang::En, Some(name)) => {
            format!("An agent (id {prefix}…) asks. Its name: {}", quote(name))
        }
        (Lang::En, None) => format!("An agent (id {prefix}…) asks."),
        (Lang::Ko, Some(name)) => {
            format!("에이전트(id {prefix}…)가 요청합니다. 이름: {}", quote(name))
        } // hangul-ok
        (Lang::Ko, None) => format!("에이전트(id {prefix}…)가 요청합니다."), // hangul-ok
    };
    if let Some(pc) = origin {
        match lang {
            Lang::En => line.push_str(&format!(" It asks after mail from PC {}.", quote(pc))),
            Lang::Ko => line.push_str(&format!(" PC {}에서 온 우편 다음의 요청입니다.", quote(pc))), // hangul-ok
        }
    }
    line
}

fn change_text(
    lang: Lang,
    asked: &Asked<'_>,
    agent_id: &str,
    agent_name: Option<&str>,
    origin: Option<&str>,
    request: &str,
) -> QuestionText {
    let l = labels(lang);
    let (heading, detail, allows): (String, String, String) = match asked {
        Asked::Add {
            peer,
            knocked,
            name_was,
        } => {
            let mut detail = peer_lines(lang, peer);
            if *knocked {
                detail.push('\n');
                detail.push_str(&pick(
                    lang,
                    "Note: that PC tried to link to this one while it was not on the trusted list.",
                    "참고: 그 PC는 신뢰 목록에 없을 때 이 PC에 연결을 시도했습니다.", // hangul-ok
                ));
            }
            if let Some(other) = name_was {
                detail.push('\n');
                detail.push_str(&match lang {
                    Lang::En => format!(
                        "Note: earlier in this run the name {} belonged to another PC (ID {}).",
                        quote(&peer.name),
                        other.grouped()
                    ),
                    Lang::Ko => format!(
                        "참고: 이번 실행 앞부분에서 이름 {}은(는) 다른 PC(ID {})의 것이었습니다.", // hangul-ok
                        quote(&peer.name),
                        other.grouped()
                    ),
                });
            }
            (
                pick(lang, "An agent asks to trust another PC", "에이전트가 다른 PC를 신뢰하자고 요청합니다"), // hangul-ok
                detail,
                pick(
                    lang,
                    "Approving adds it to this PC's trusted list: that PC may link to this one and send mail to its agents.",
                    "승인하면 이 PC의 신뢰 목록에 추가됩니다: 그 PC가 이 PC에 연결해 이 PC의 에이전트에게 우편을 보낼 수 있습니다.", // hangul-ok
                ),
            )
        }
        Asked::SetNetwork { from, patch } => {
            // Every field the request names, from → to — also one equal now: the human sees each value approving will write.
            let mut lines = Vec::new();
            if let Some(port) = patch.port {
                lines.push(format!("{}: {} → {}", l.port, port_text(lang, from.port), port_text(lang, port)));
            }
            if let Some(relay) = &patch.relay {
                lines.push(format!("{}: {} → {}", l.relay, helper_text(lang, &from.relay), helper_text(lang, relay)));
            }
            if let Some(lookup) = &patch.address_lookup {
                lines.push(format!(
                    "{}: {} → {}",
                    l.lookup,
                    helper_text(lang, &from.address_lookup),
                    helper_text(lang, lookup)
                ));
            }
            if let Some(mdns) = patch.mdns {
                lines.push(format!("{}: {} → {}", l.mdns, on_off(lang, from.mdns), on_off(lang, mdns)));
            }
            (
                pick(
                    lang,
                    "An agent asks to change how this PC reaches other PCs",
                    "에이전트가 이 PC가 다른 PC에 닿는 방법을 바꾸자고 요청합니다", // hangul-ok
                ),
                lines.join("\n"),
                pick(
                    lang,
                    "Approving restarts the peer link with these settings. Relay and address-lookup servers see this PC's ID and public address.",
                    "승인하면 이 설정으로 피어 링크가 다시 시작됩니다. 릴레이 서버와 주소 조회 서버는 이 PC의 ID와 공인 주소를 봅니다.", // hangul-ok
                ),
            )
        }
        Asked::RegenerateKey {
            instead_of_adopting: false,
        } => (
            pick(lang, "An agent asks to make a new identity key for this PC", "에이전트가 이 PC의 새 신원 키를 만들자고 요청합니다"), // hangul-ok
            String::new(),
            pick(
                lang,
                "Approving gives this PC a new ID: every other PC must remove the old ID and pair with the new one.",
                "승인하면 이 PC의 ID가 바뀝니다: 다른 PC마다 옛 ID를 지우고 새 ID와 다시 짝지어야 합니다.", // hangul-ok
            ),
        ),
        Asked::RegenerateKey {
            instead_of_adopting: true,
        } => (
            pick(
                lang,
                "An agent asks to give this PC a fresh identity instead of adopting the saved one",
                "에이전트가 저장된 신원을 넘겨받는 대신 이 PC에 새 신원을 만들자고 요청합니다", // hangul-ok
            ),
            String::new(),
            pick(
                lang,
                "Approving abandons the identity this account saved under another machine id, and every pairing it has: the other PCs must pair with the new ID.",
                "승인하면 이 계정이 다른 기기 ID로 저장해 둔 신원과 그 짝지음을 모두 버립니다: 다른 PC들이 새 ID와 다시 짝지어야 합니다.", // hangul-ok
            ),
        ),
        Asked::Rename {
            id,
            from_name,
            to_name,
            from_address,
            to_address,
        } => {
            let mut detail = format!("{}: {}", l.id, id.grouped());
            let old = from_name.map(quote).unwrap_or_else(|| pick(lang, "(unknown)", "(알 수 없음)")); // hangul-ok
            if let Some(new) = to_name {
                detail.push_str(&format!("\n{}: {old} → {}", l.name, quote(new)));
            }
            if let Some(to) = to_address {
                detail.push_str(&format!(
                    "\n{}: {} → {}",
                    l.address,
                    address_text(lang, *from_address),
                    address_text(lang, *to)
                ));
            }
            let old_bare = from_name.map(|n| visible(n, QUOTE_CAP)).unwrap_or_default();
            (
                pick(lang, "An agent asks to rename a trusted PC", "에이전트가 신뢰하는 PC의 이름을 바꾸자고 요청합니다"), // hangul-ok
                detail,
                match lang {
                    Lang::En => format!(
                        "Approving changes the name mail uses for that PC: addresses written name@{old_bare} stop resolving."
                    ),
                    Lang::Ko => format!(
                        "승인하면 그 PC에 우편을 보낼 때 쓰는 이름이 바뀝니다: name@{old_bare}(으)로 쓴 주소는 더 이상 닿지 않습니다." // hangul-ok
                    ),
                },
            )
        }
    };
    let mut body = agent_line(lang, agent_id, agent_name, origin);
    if !detail.is_empty() {
        body.push_str("\n\n");
        body.push_str(&detail);
    }
    let footer = match lang {
        Lang::En => format!("Request {request}."),
        Lang::Ko => format!("요청 {request}."), // hangul-ok
    };
    body.push_str(&format!("\n\n{allows}\n\n{footer}"));
    QuestionText {
        title: pick(
            lang,
            "Engram — approve a trust change?",
            "Engram — 신뢰 변경을 승인할까요?",
        ), // hangul-ok
        heading,
        body,
        approve: pick(lang, "Approve", "승인"), // hangul-ok
        deny: pick(lang, "Deny", "거절"),       // hangul-ok
        deny_and_stop: pick(
            lang,
            "Deny and stop asking this agent",
            "거절하고 이 에이전트 요청 그만 받기",
        ), // hangul-ok
    }
}

/// An agent's request, in every language.
pub(crate) fn change_question(
    asked: &Asked<'_>,
    agent_id: &str,
    agent_name: Option<&str>,
    origin: Option<&str>,
    request: &str,
) -> Question {
    let [en, ko] =
        LANGS.map(|lang| change_text(lang, asked, agent_id, agent_name, origin, request));
    Question {
        kind: asked.kind(),
        en,
        ko,
    }
}

/// The accept-list question: the list was changed outside Engram (`corrupt` = the file is damaged and accepting starts an empty list).
pub(crate) fn accept_list_question(
    corrupt: bool,
    peers: &[TrustedPeer],
    network: Option<&NetworkSettings>,
    request: &str,
) -> Question {
    let [en, ko] = LANGS.map(|lang| {
        let heading = match corrupt {
            false => pick(lang, "The trusted list was changed outside Engram", "신뢰 목록이 Engram 밖에서 바뀌었습니다"), // hangul-ok
            true => pick(lang, "The trusted list file is damaged", "신뢰 목록 파일이 손상되었습니다"), // hangul-ok
        };
        let mut body = match lang {
            Lang::En => format!("Trusted PCs: {}", peers.len()),
            Lang::Ko => format!("신뢰하는 PC: {}대", peers.len()), // hangul-ok
        };
        for peer in peers.iter().take(LISTED_PCS) {
            body.push_str("\n\n");
            body.push_str(&peer_lines(lang, peer));
        }
        let more = peers.len().saturating_sub(LISTED_PCS);
        if more > 0 {
            body.push_str("\n\n");
            body.push_str(&match lang {
                Lang::En => format!("{more} more — review them in the app or with peer.list before accepting."),
                Lang::Ko => format!("외 {more}대 — 받아들이기 전에 앱이나 peer.list로 확인하세요."), // hangul-ok
            });
        }
        if let Some(network) = network {
            body.push_str("\n\n");
            body.push_str(&network_lines(lang, network));
        }
        let allows = match corrupt {
            false => pick(
                lang,
                "Accepting turns peer links on with this list as it is now. Peer links stay off until it is accepted.",
                "받아들이면 지금 이 목록 그대로 피어 링크가 켜집니다. 받아들이기 전에는 피어 링크가 꺼져 있습니다.", // hangul-ok
            ),
            true => pick(
                lang,
                "Accepting starts an empty list with default network settings (the damaged file is set aside). Peer links stay off until then.",
                "받아들이면 빈 목록과 기본 네트워크 설정으로 시작합니다(손상된 파일은 따로 보관). 그 전에는 피어 링크가 꺼져 있습니다.", // hangul-ok
            ),
        };
        let footer = match lang {
            Lang::En => format!("Request {request}."),
            Lang::Ko => format!("요청 {request}."), // hangul-ok
        };
        QuestionText {
            title: pick(lang, "Engram — accept the trusted list?", "Engram — 신뢰 목록을 받아들일까요?"), // hangul-ok
            heading,
            body: format!("{body}\n\n{allows}\n\n{footer}"),
            approve: pick(lang, "Accept the list", "목록 받아들이기"), // hangul-ok
            deny: pick(lang, "Not now", "나중에"),                    // hangul-ok
            deny_and_stop: pick(lang, "Not this run", "이번 실행에서는 묻지 않기"), // hangul-ok
        }
    });
    Question {
        kind: "acceptList",
        en,
        ko,
    }
}

/// The adopt question for one machine id (`others` = how many more machine ids hold an identity of this account). Its third choice is
///   "Start fresh".
pub(crate) fn adopt_question(machine: &str, others: usize, request: &str) -> Question {
    let [en, ko] = LANGS.map(|lang| {
        let mut body = match lang {
            Lang::En => format!("Machine id: {}", quote(machine)),
            Lang::Ko => format!("기기 ID: {}", quote(machine)), // hangul-ok
        };
        if others > 0 {
            body.push('\n');
            body.push_str(&match lang {
                Lang::En => format!("{others} other machine ids also hold an identity of this account — this question asks about one at a time."),
                Lang::Ko => format!("다른 기기 ID {others}개에도 이 계정의 신원이 있습니다 — 하나씩 묻습니다."), // hangul-ok
            });
        }
        let explain = pick(
            lang,
            "Adopt makes it this PC's identity (its ID and trusted list). If that identity still runs on another PC, two PCs would share one ID — then choose Start fresh: a new ID that every other PC must pair with.",
            "넘겨받으면 그 신원(ID와 신뢰 목록)이 이 PC의 것이 됩니다. 그 신원이 다른 PC에서 아직 쓰이고 있다면 두 PC가 한 ID를 함께 쓰게 됩니다 — 그럴 땐 새로 시작을 고르세요: 다른 PC들이 새 ID와 다시 짝지어야 합니다.", // hangul-ok
        );
        let footer = match lang {
            Lang::En => format!("Request {request}."),
            Lang::Ko => format!("요청 {request}."), // hangul-ok
        };
        QuestionText {
            title: pick(lang, "Engram — adopt this PC's identity?", "Engram — 이 PC의 신원을 넘겨받을까요?"), // hangul-ok
            heading: pick(
                lang,
                "This Windows account has a peer identity saved under another machine id",
                "이 Windows 계정에 다른 기기 ID로 저장된 피어 신원이 있습니다", // hangul-ok
            ),
            body: format!("{body}\n\n{explain}\n\n{footer}"),
            approve: pick(lang, "Adopt", "넘겨받기"), // hangul-ok
            deny: pick(lang, "Not this run", "이번 실행에서는 묻지 않기"), // hangul-ok
            deny_and_stop: pick(lang, "Start fresh", "새로 시작"), // hangul-ok
        }
    });
    Question {
        kind: "adopt",
        en,
        ko,
    }
}
// hangul-ok-end
