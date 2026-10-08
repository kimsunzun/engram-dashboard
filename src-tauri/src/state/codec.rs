//! `state.json` 의 글 ↔ [`StateFile`] — 읽기 관용과 쓰기 상한(크기 · 중첩 — TRD S21-storage §6-2 · §6-4 I4).
//! 파일은 열지 않는다 — 읽기 · 쓰기는 부르는 쪽이 `fsutil` 로 한다.
//!
//! 읽기는 세 층으로 접는다:
//! - **통째로 못 쓴다**([`Unusable`]) — JSON 이 아니다 · 머리(`version` · `saved_at_ms` · `clean_exit` ·
//!   `resolved_crash_copy` · `windows` 배열)를 못 읽는다 · 이 셸보다 새 `version` 이다. 원문의 상한 초과 ·
//!   UTF-8 아님도 이쪽이다([`unusable_read`]).
//! - **그 창만 건너뛴다** — 모르는 `kind` · 못 읽는 창 칸(탭 칸 포함).
//! - **그 탭만 건너뛴다** — 못 읽는 탭(모르는 노드 종류 · 객체가 아닌 슬롯 내용 등).
//!
//! 창의 `theme` 이 모르는 값이면 창은 살리고 그 칸만 `None`(전역 테마)으로 접는다. `bounds` 가 없거나 `null` 이면
//! `None`(자리를 본 적 없는 창)이다 — 경고가 아니다. 건너뛰고 접은 것은 [`DecodeWarning`] 으로 돌려준다 — 로그는
//! 부르는 쪽이 낸다.

use std::fmt;
use std::io;

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::schema::{
    Bounds, StateFile, TabEntry, TabStrip, WindowEntry, WindowKind, STATE_VERSION,
};
use crate::theme::UiTheme;

/// 읽기 상한이자 쓰기 상한 — 넘는 글을 쓰면 다음 부팅이 그 파일 전체를 못 쓴다고 접는다(I4). 부르는 쪽은 이
/// 값으로 `fsutil::read_file_capped` 한다.
pub const STATE_READ_CAP: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unusable {
    /// 상한 초과 · UTF-8 아님 — 원문을 못 가져왔다([`unusable_read`]).
    #[error("원문을 못 읽는다: {0}")]
    Unreadable(String),
    #[error("JSON 이 아니다: {0}")]
    NotJson(String),
    /// JSON 이지만 상태 파일의 머리를 못 읽는다.
    #[error("상태 파일 모양이 아니다: {0}")]
    NotStateFile(String),
    #[error("이 셸보다 새 형식이다(version {found} — 이 셸은 {STATE_VERSION} 까지)")]
    NewerVersion { found: u64 },
}

/// [`encode`] 가 쓰기를 거절한 사유 — 둘 다 그대로 쓰면 다음 부팅이 그 파일 전체를 못 쓴다고 접는다.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("쓸 원문({len} 바이트)이 상한 {STATE_READ_CAP} 바이트를 넘는다")]
    TooLarge { len: usize },
    /// [`decode`] 와 같은 파서로 다시 읽히지 않는다 — 닿는 원인은 serde_json 의 중첩 상한(128 단)이다(같은 슬롯을
    /// 거듭 나눈 분할 트리). 바이트 상한 검사는 이것을 못 잡는다.
    #[error("쓸 원문이 다시 읽히지 않는다: {reason}")]
    Unparsable { reason: String },
}

/// 읽기에서 건너뛰거나 접은 것 하나 — 파일 전체는 쓸 수 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeWarning {
    /// `index` = `windows` 배열 안의 자리. `id` = 읽을 수 있었다면 그 창의 `id`.
    WindowSkipped {
        index: usize,
        id: Option<String>,
        reason: String,
    },
    /// `index` = 그 창 `tabs` 배열 안의 자리.
    TabSkipped {
        window_id: String,
        index: usize,
        reason: String,
    },
    /// `raw` = 받은 값의 JSON 원문.
    ThemeDropped { window_id: String, raw: String },
}

impl fmt::Display for DecodeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeWarning::WindowSkipped { index, id, reason } => match id {
                Some(id) => write!(f, "창 {index}번({id})을 건너뛴다: {reason}"),
                None => write!(f, "창 {index}번을 건너뛴다: {reason}"),
            },
            DecodeWarning::TabSkipped {
                window_id,
                index,
                reason,
            } => write!(f, "창 {window_id} 의 탭 {index}번을 건너뛴다: {reason}"),
            DecodeWarning::ThemeDropped { window_id, raw } => {
                write!(
                    f,
                    "창 {window_id} 의 모르는 테마 {raw} — 전역 테마를 따른다"
                )
            }
        }
    }
}

pub fn decode(text: &str) -> Result<(StateFile, Vec<DecodeWarning>), Unusable> {
    // 메모장 · PowerShell 은 UTF-8 로 저장하면 BOM 을 붙인다 — 손으로 고친 파일이 그것만으로 못 쓰게 되지 않게.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let doc = parse(text).map_err(|e| Unusable::NotJson(e.to_string()))?;
    if !doc.is_object() {
        return Err(Unusable::NotStateFile("JSON 객체가 아니다".to_string()));
    }
    // 머리보다 먼저 본다 — 새 형식은 머리 모양부터 다를 수 있고, 그때도 「새 형식」으로 알려야 한다.
    match doc.get("version").and_then(version_number) {
        Some(found) if found == u64::from(STATE_VERSION) => {}
        Some(found) if found > u64::from(STATE_VERSION) => {
            return Err(Unusable::NewerVersion { found })
        }
        _ => {
            return Err(Unusable::NotStateFile(
                "`version` 이 없거나 이 셸이 쓴 적 없는 값이다".to_string(),
            ))
        }
    }
    let head = Head::deserialize(doc).map_err(|e| Unusable::NotStateFile(e.to_string()))?;

    let mut warnings = Vec::new();
    let windows = head
        .windows
        .into_iter()
        .enumerate()
        .filter_map(|(index, raw)| decode_window(index, raw, &mut warnings))
        .collect();
    let state = StateFile {
        version: STATE_VERSION,
        saved_at_ms: head.saved_at_ms,
        clean_exit: head.clean_exit,
        resolved_crash_copy: head.resolved_crash_copy,
        windows,
    };
    Ok((state, warnings))
}

pub fn encode(state: &StateFile) -> Result<String, EncodeError> {
    // 사람 · 에이전트가 열어 보는 파일이라 줄을 나눈다.
    let mut text = serde_json::to_string_pretty(state)
        .expect("문자열 키와 실패하지 않는 직렬화기뿐이라 JSON 직렬화가 실패할 수 없다");
    text.push('\n');
    if text.len() as u64 > STATE_READ_CAP {
        return Err(EncodeError::TooLarge { len: text.len() });
    }
    parse(&text).map_err(|e| EncodeError::Unparsable {
        reason: e.to_string(),
    })?;
    Ok(text)
}

/// [`decode`] 와 [`encode`] 의 다시 읽기가 함께 쓰는 파서 — 둘이 같은 중첩 상한을 본다.
// ★`IgnoredAny` 로 바꾸지 말 것 · 상한을 끄지 말 것★ — `IgnoredAny` 로 건너뛰는 경로는 중첩 깊이를 재지 않아,
//   다시 읽기를 통과한 글을 `decode` 가 못 읽는다. 상한을 끄면 깊은 중첩의 파일이 스택을 넘친다.
fn parse(text: &str) -> serde_json::Result<Value> {
    serde_json::from_str(text)
}

/// 정수 값인 실수(`1.0`)도 받는다 — JSON 도구가 수를 실수로 다시 쓰기도 한다(설정 파일과 같은 관용). 음수 ·
/// 소수 · `u64` 범위 밖 · 수가 아닌 값은 `None`.
fn version_number(raw: &Value) -> Option<u64> {
    // 2^64 — `u64::MAX as f64` 와 같은 값이라 그 자체는 범위 밖이다. `as u64` 는 범위 밖을 u64::MAX 로 눌러
    //   없는 「새 형식」을 지어내므로 자르기 전에 거른다.
    const U64_END: f64 = 18_446_744_073_709_551_616.0;
    raw.as_u64().or_else(|| {
        let float = raw.as_f64()?;
        (0.0..U64_END)
            .contains(&float)
            .then_some(float)
            .filter(|float| float.fract() == 0.0)
            .map(|float| float as u64)
    })
}

/// `fsutil::read_file_capped` 의 오류 중 파일 내용이 못 쓸 것인 경우 — 상한 초과 · UTF-8 아님(`InvalidData`).
///
/// `None` = 내용이 아니라 읽기 자체가 실패했다(없음 · 잠김 · 권한). ★못 쓸 파일과 섞지 않는다★ — 잠깐 잠긴
/// 멀쩡한 파일을 떠 두거나 덮지 않는다(TRD §6-2 I3).
pub fn unusable_read(err: &io::Error) -> Option<Unusable> {
    (err.kind() == io::ErrorKind::InvalidData).then(|| Unusable::Unreadable(err.to_string()))
}

/// 크래시 사본의 신원 — [`StateFile::resolved_crash_copy`] 에 싣는 값. 읽은 원문 바이트 그대로를 잰다 — 부팅과
/// 기록기가 모두 이 함수로 재야 같은 사본을 같다고 본다.
pub fn crash_copy_hash(text: &str) -> String {
    crate::fsutil::fnv1a_hex(text.as_bytes())
}

#[derive(Deserialize)]
struct Head {
    saved_at_ms: u64,
    clean_exit: bool,
    resolved_crash_copy: Option<String>,
    windows: Vec<Value>,
}

/// [`WindowKind`] 의 `kind` 철자 — 같은 `rename_all` 이라 같은 철자다(왕복 시험이 맞댄다).
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindTag {
    Main,
    Popout,
}

#[derive(Deserialize)]
struct WindowHead {
    id: String,
    #[serde(default)]
    theme: Value,
    // 없음 · `null` = `None`. 객체가 아니거나 칸이 수가 아니면(유한하지 않은 실수는 `null` 로 나간다) 그 창의 읽기
    // 실패다 — 이 판의 쓰기는 그런 값을 내지 않는다(`WindowBounds` 가 유한한 값만 쥔다).
    #[serde(default)]
    bounds: Option<Bounds>,
    maximized: bool,
}

#[derive(Deserialize)]
struct StripHead {
    active_tab: Uuid,
    tabs: Vec<Value>,
}

fn decode_window(
    index: usize,
    raw: Value,
    warnings: &mut Vec<DecodeWarning>,
) -> Option<WindowEntry> {
    let id = raw.get("id").and_then(Value::as_str).map(str::to_string);
    let mut skip = |reason: String| {
        warnings.push(DecodeWarning::WindowSkipped {
            index,
            id: id.clone(),
            reason,
        });
    };
    let kind = match raw.get("kind") {
        None => {
            skip("`kind` 가 없다".to_string());
            return None;
        }
        Some(kind) => match KindTag::deserialize(kind) {
            Ok(kind) => kind,
            Err(_) => {
                skip(format!("모르는 창 종류 {kind}"));
                return None;
            }
        },
    };
    let head = match WindowHead::deserialize(&raw) {
        Ok(head) => head,
        Err(e) => {
            skip(e.to_string());
            return None;
        }
    };
    let wrap: fn(TabStrip) -> WindowKind = match kind {
        KindTag::Main => WindowKind::Main,
        KindTag::Popout => WindowKind::Popout,
    };
    let kind = match decode_strip(&head.id, raw, warnings) {
        Ok(strip) => wrap(strip),
        Err(reason) => {
            warnings.push(DecodeWarning::WindowSkipped {
                index,
                id: Some(head.id),
                reason,
            });
            return None;
        }
    };
    let theme = match &head.theme {
        Value::Null => None,
        raw => match raw.as_str().and_then(UiTheme::from_wire) {
            Some(theme) => Some(theme),
            None => {
                warnings.push(DecodeWarning::ThemeDropped {
                    window_id: head.id.clone(),
                    raw: raw.to_string(),
                });
                None
            }
        },
    };
    Some(WindowEntry {
        id: head.id,
        kind,
        theme,
        bounds: head.bounds,
        maximized: head.maximized,
    })
}

/// `Err` = 탭 칸 자체(`active_tab` · `tabs` 배열)를 못 읽는다 — 그 창째 건너뛴다.
fn decode_strip(
    window_id: &str,
    raw: Value,
    warnings: &mut Vec<DecodeWarning>,
) -> Result<TabStrip, String> {
    let head = StripHead::deserialize(raw).map_err(|e| e.to_string())?;
    let tabs = head
        .tabs
        .into_iter()
        .enumerate()
        .filter_map(|(index, raw)| match TabEntry::deserialize(raw) {
            Ok(tab) => Some(tab),
            Err(e) => {
                warnings.push(DecodeWarning::TabSkipped {
                    window_id: window_id.to_string(),
                    index,
                    reason: e.to_string(),
                });
                None
            }
        })
        .collect();
    Ok(TabStrip {
        active_tab: head.active_tab,
        tabs,
    })
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::{json, Value};
    use uuid::Uuid;

    use super::{
        crash_copy_hash, decode, encode, unusable_read, DecodeWarning, EncodeError, Unusable,
        STATE_READ_CAP,
    };
    use crate::layout::{SlotContent, SplitDir};
    use crate::state::schema::{
        Bounds, PersistedContent, PersistedNode, StateFile, TabEntry, TabStrip, WindowEntry,
        WindowKind, STATE_VERSION,
    };
    use crate::theme::UiTheme;

    fn bounds() -> Bounds {
        Bounds {
            x: -1920.0,
            y: 60.5,
            w: 1280.0,
            h: 800.0,
        }
    }

    fn known(content: SlotContent) -> PersistedNode {
        PersistedNode::Slot {
            id: Uuid::new_v4(),
            content: PersistedContent::Known(content),
        }
    }

    fn tab(name: &str, layout: PersistedNode) -> TabEntry {
        TabEntry {
            id: Uuid::new_v4(),
            name: name.to_string(),
            focused_slot_id: None,
            layout,
        }
    }

    fn full_state() -> StateFile {
        let focused = Uuid::new_v4();
        let first = TabEntry {
            id: Uuid::new_v4(),
            name: "View 1".to_string(),
            focused_slot_id: Some(focused),
            layout: PersistedNode::Split {
                id: Uuid::new_v4(),
                dir: SplitDir::LeftRight,
                ratio: 0.25,
                a: Box::new(PersistedNode::Slot {
                    id: focused,
                    content: PersistedContent::Known(SlotContent::Agent {
                        agent_id: "a-1".to_string(),
                    }),
                }),
                b: Box::new(PersistedNode::Split {
                    id: Uuid::new_v4(),
                    dir: SplitDir::TopBottom,
                    ratio: 0.5,
                    a: Box::new(known(SlotContent::Usage {
                        show_claude: true,
                        show_codex: false,
                    })),
                    b: Box::new(known(SlotContent::Empty)),
                }),
            },
        };
        let second = tab("탭 둘", known(SlotContent::AgentList));
        let popout = tab("View 3", known(SlotContent::PresetPalette));
        StateFile {
            version: STATE_VERSION,
            saved_at_ms: 1_759_400_000_000,
            clean_exit: false,
            resolved_crash_copy: Some(crash_copy_hash("{\"version\":1}")),
            windows: vec![
                WindowEntry {
                    id: "main".to_string(),
                    kind: WindowKind::Main(TabStrip {
                        active_tab: second.id,
                        tabs: vec![first, second],
                    }),
                    theme: Some(UiTheme::Light),
                    bounds: Some(bounds()),
                    maximized: true,
                },
                WindowEntry {
                    id: Uuid::new_v4().to_string(),
                    kind: WindowKind::Popout(TabStrip {
                        active_tab: popout.id,
                        tabs: vec![popout],
                    }),
                    theme: None,
                    bounds: Some(bounds()),
                    maximized: false,
                },
            ],
        }
    }

    fn main_strip(state: &mut StateFile) -> &mut TabStrip {
        match &mut state.windows[0].kind {
            WindowKind::Main(strip) => strip,
            other => panic!("main 창이어야 한다: {other:?}"),
        }
    }

    fn json_of(state: &StateFile) -> Value {
        serde_json::from_str(&encode(state).unwrap()).unwrap()
    }

    fn decode_value(doc: &Value) -> (StateFile, Vec<DecodeWarning>) {
        decode(&doc.to_string()).unwrap()
    }

    fn window(id: &str, kind: &str) -> Value {
        json!({
            "id": id, "kind": kind, "theme": null,
            "bounds": { "x": 0, "y": 0, "w": 800, "h": 600 }, "maximized": false,
        })
    }

    fn tabbed(id: &str, kind: &str, tabs: Vec<Value>) -> Value {
        let mut window = window(id, kind);
        window["active_tab"] = json!(Uuid::new_v4());
        window["tabs"] = Value::Array(tabs);
        window
    }

    /// 읽히는 창 — 빈 탭 하나를 든 팝아웃(코덱은 팝아웃 id 가 UUID 인지 보지 않는다).
    fn readable(id: &str) -> Value {
        tabbed(id, "popout", vec![tab_json(json!({ "type": "empty" }))])
    }

    fn tab_json(content: Value) -> Value {
        json!({
            "id": Uuid::new_v4(), "name": "t", "focused_slot_id": null,
            "layout": { "type": "slot", "id": Uuid::new_v4(), "content": content },
        })
    }

    fn file(windows: Vec<Value>) -> Value {
        json!({ "version": 1, "saved_at_ms": 5, "clean_exit": true, "windows": windows })
    }

    fn skipped_windows(warnings: &[DecodeWarning]) -> Vec<usize> {
        warnings
            .iter()
            .map(|w| match w {
                DecodeWarning::WindowSkipped { index, .. } => *index,
                other => panic!("창 건너뛰기여야 한다: {other}"),
            })
            .collect()
    }

    // ── 왕복 ──

    #[test]
    fn a_full_file_round_trips() {
        let state = full_state();
        let text = encode(&state).unwrap();
        let (back, warnings) = decode(&text).unwrap();
        assert_eq!(warnings, vec![]);
        assert_eq!(back, state);
        assert_eq!(encode(&back).unwrap(), text);
    }

    #[test]
    fn the_file_has_the_trd_shape() {
        let doc = json_of(&full_state());
        let mut top: Vec<&str> = doc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        top.sort_unstable();
        assert_eq!(
            top,
            [
                "clean_exit",
                "resolved_crash_copy",
                "saved_at_ms",
                "version",
                "windows"
            ]
        );
        assert_eq!(doc["version"], 1);
        let main = &doc["windows"][0];
        assert_eq!(main["kind"], "main");
        assert_eq!(main["theme"], "light");
        assert_eq!(
            main["bounds"],
            json!({ "x": -1920.0, "y": 60.5, "w": 1280.0, "h": 800.0 })
        );
        assert_eq!(main["maximized"], true);
        assert!(main["active_tab"].is_string());
        assert_eq!(main["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(main["tabs"][0]["layout"]["type"], "split");
        assert_eq!(main["tabs"][0]["layout"]["dir"], "left_right");

        let popout = &doc["windows"][1];
        assert_eq!(popout["kind"], "popout");
        assert_eq!(popout["theme"], Value::Null);
    }

    #[test]
    fn resolved_crash_copy_is_written_only_when_present() {
        let mut state = full_state();
        let hash = state.resolved_crash_copy.clone().unwrap();
        assert_eq!(json_of(&state)["resolved_crash_copy"], json!(hash));

        state.resolved_crash_copy = None;
        let doc = json_of(&state);
        assert!(!doc.as_object().unwrap().contains_key("resolved_crash_copy"));
        assert_eq!(decode_value(&doc).0.resolved_crash_copy, None);
    }

    #[test]
    fn crash_copy_hash_is_the_fnv1a_of_the_raw_text() {
        let text = "\u{feff}{\"version\":1}\r\n";
        assert_eq!(
            crash_copy_hash(text),
            crate::fsutil::fnv1a_hex(text.as_bytes())
        );
        assert_ne!(crash_copy_hash(text), crash_copy_hash(&text[3..]));
    }

    // ── 모르는 슬롯 내용(ADR-0060) ──

    #[test]
    fn unknown_slot_content_survives_decode_and_encode() {
        let foreign = json!({
            "type": "file_tree", "root": "C:\\work",
            "opts": { "depth": 3, "list": [1, 2.5, null, true, "x"], "nested": { "k": [] } },
        });
        let malformed_known = json!({ "type": "agent", "agent_ref": 7 });
        let doc = file(vec![tabbed(
            "main",
            "main",
            vec![tab_json(foreign.clone()), tab_json(malformed_known.clone())],
        )]);
        let (mut state, warnings) = decode_value(&doc);
        assert_eq!(warnings, vec![]);
        for tab in &main_strip(&mut state).tabs {
            let PersistedNode::Slot { content, .. } = &tab.layout else {
                panic!("슬롯이어야 한다")
            };
            assert!(matches!(content, PersistedContent::Unknown(_)));
        }

        let back = json_of(&state);
        assert_eq!(back["windows"][0]["tabs"][0]["layout"]["content"], foreign);
        assert_eq!(
            back["windows"][0]["tabs"][1]["layout"]["content"],
            malformed_known
        );
        assert_eq!(back["windows"][0]["tabs"], doc["windows"][0]["tabs"]);
    }

    // ── 창 · 탭 건너뛰기 ──

    #[test]
    fn an_unknown_window_kind_skips_only_that_window() {
        let popout = Uuid::new_v4().to_string();
        let doc = file(vec![
            tabbed("main", "main", vec![tab_json(json!({ "type": "empty" }))]),
            window("dock-1", "dock"),
            tabbed(
                &popout,
                "popout",
                vec![tab_json(json!({ "type": "agent_list" }))],
            ),
        ]);
        let (state, warnings) = decode_value(&doc);
        let ids: Vec<&str> = state.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["main", popout.as_str()]);
        assert!(matches!(
            warnings.as_slice(),
            [DecodeWarning::WindowSkipped { index: 1, id: Some(id), .. }] if id == "dock-1"
        ));
    }

    /// ADR-0225: 걷어 낸 트리 전용 창의 옛 항목(`kind: "tree"`)은 모르는 창 종류로 건너뛰고 나머지는 그대로 읽는다 —
    /// 옛 파일 하나 때문에 저장된 화면을 잃지 않는다.
    #[test]
    fn a_retired_tree_window_entry_is_skipped_and_the_rest_is_kept() {
        let mut tree = window("agent-tree", "tree");
        tree["theme"] = json!("e-ink");
        let doc = file(vec![
            tabbed("main", "main", vec![tab_json(json!({ "type": "empty" }))]),
            tree,
        ]);
        let (state, warnings) = decode_value(&doc);
        let ids: Vec<&str> = state.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["main"]);
        assert!(
            matches!(
                warnings.as_slice(),
                [DecodeWarning::WindowSkipped { index: 1, id: Some(id), reason }]
                    if id == "agent-tree" && reason.contains("모르는 창 종류")
            ),
            "{warnings:?}"
        );
    }

    #[test]
    fn an_unreadable_window_is_skipped() {
        let mut no_kind = readable("w-0");
        no_kind.as_object_mut().unwrap().remove("kind");
        let mut no_maximized = readable("w-1");
        no_maximized.as_object_mut().unwrap().remove("maximized");
        let mut bad_active = tabbed("w-2", "popout", vec![tab_json(json!({ "type": "empty" }))]);
        bad_active["active_tab"] = json!("not-a-uuid");
        let no_tabs = window("w-3", "main");
        let doc = file(vec![
            no_kind,
            no_maximized,
            bad_active,
            no_tabs,
            json!("not a window"),
            readable("w-5"),
        ]);
        let (state, warnings) = decode_value(&doc);
        let ids: Vec<&str> = state.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["w-5"]);
        assert_eq!(skipped_windows(&warnings), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_bad_tab_skips_only_that_tab() {
        let good = tab_json(json!({ "type": "agent", "agent_id": "a-1" }));
        let mut bad_id = tab_json(json!({ "type": "empty" }));
        bad_id["id"] = json!("not-a-uuid");
        let mut unknown_node = tab_json(json!({ "type": "empty" }));
        unknown_node["layout"] = json!({ "type": "grid", "id": Uuid::new_v4(), "cells": [] });
        let not_object_content = tab_json(json!("empty"));
        let main = tabbed(
            "main",
            "main",
            vec![bad_id, good.clone(), unknown_node, not_object_content],
        );
        let (mut state, warnings) = decode_value(&file(vec![main.clone()]));

        let strip = main_strip(&mut state);
        assert_eq!(strip.tabs.len(), 1);
        assert_eq!(serde_json::to_value(&strip.tabs[0]).unwrap(), good);
        assert_eq!(strip.active_tab.to_string(), main["active_tab"]);
        let skipped: Vec<usize> = warnings
            .iter()
            .map(|w| match w {
                DecodeWarning::TabSkipped {
                    window_id, index, ..
                } if window_id == "main" => *index,
                other => panic!("main 의 탭 건너뛰기여야 한다: {other}"),
            })
            .collect();
        assert_eq!(skipped, [0, 2, 3]);
    }

    #[test]
    fn an_unknown_theme_keeps_the_window_and_drops_the_theme() {
        let mut odd = readable("w-0");
        odd["theme"] = json!("sepia");
        let mut missing = readable("w-1");
        missing.as_object_mut().unwrap().remove("theme");
        let mut set = readable("w-2");
        set["theme"] = json!("dark");
        let (state, warnings) = decode_value(&file(vec![odd, missing, set]));
        let themes: Vec<Option<UiTheme>> = state.windows.iter().map(|w| w.theme).collect();
        assert_eq!(themes, [None, None, Some(UiTheme::Dark)]);
        assert_eq!(
            warnings,
            vec![DecodeWarning::ThemeDropped {
                window_id: "w-0".to_string(),
                raw: "\"sepia\"".to_string(),
            }]
        );
    }

    #[test]
    fn missing_or_null_bounds_read_as_no_place() {
        let mut missing = readable("w-0");
        missing.as_object_mut().unwrap().remove("bounds");
        let mut null = readable("w-1");
        null["bounds"] = Value::Null;
        // 유한하지 않은 실수는 직렬화에서 `null` 이 된다 — 칸 하나가 `null` 인 자리는 못 읽는 창이다.
        let mut holed = readable("w-2");
        holed["bounds"] = json!({ "x": null, "y": 0, "w": 800, "h": 600 });
        let kept = readable("w-3");
        let (state, warnings) = decode_value(&file(vec![missing, null, holed, kept]));
        let places: Vec<(&str, Option<Bounds>)> = state
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.bounds))
            .collect();
        assert_eq!(
            places,
            [
                ("w-0", None),
                ("w-1", None),
                (
                    "w-3",
                    Some(Bounds {
                        x: 0.0,
                        y: 0.0,
                        w: 800.0,
                        h: 600.0
                    })
                )
            ]
        );
        assert_eq!(skipped_windows(&warnings), [2]);
    }

    #[test]
    fn a_window_without_a_place_writes_null_and_round_trips() {
        let mut state = full_state();
        state.windows[0].bounds = None;
        let doc = json_of(&state);
        assert_eq!(doc["windows"][0]["bounds"], Value::Null);
        let (back, warnings) = decode_value(&doc);
        assert_eq!(warnings, vec![]);
        assert_eq!(back, state);
    }

    // ── 통째로 못 쓰는 파일 ──

    #[test]
    fn a_newer_version_is_unusable() {
        let mut doc = file(vec![]);
        doc["version"] = json!(2);
        assert_eq!(
            decode(&doc.to_string()),
            Err(Unusable::NewerVersion { found: 2 })
        );
        // 머리 모양이 달라도 「새 형식」이다.
        assert_eq!(
            decode(r#"{"version":7,"everything":"else"}"#),
            Err(Unusable::NewerVersion { found: 7 })
        );
        doc["version"] = json!(2.0);
        assert_eq!(
            decode(&doc.to_string()),
            Err(Unusable::NewerVersion { found: 2 })
        );
    }

    #[test]
    fn an_integer_valued_float_version_reads_as_that_integer() {
        let mut doc = file(vec![readable("w-0")]);
        doc["version"] = json!(1.0);
        assert!(doc.to_string().contains("\"version\":1.0"));
        let (state, warnings) = decode_value(&doc);
        assert_eq!((state.version, state.windows.len()), (STATE_VERSION, 1));
        assert_eq!(warnings, vec![]);
    }

    #[test]
    fn not_json_is_unusable() {
        assert!(matches!(decode("{BROKEN"), Err(Unusable::NotJson(_))));
        assert!(matches!(decode(""), Err(Unusable::NotJson(_))));
    }

    #[test]
    fn json_without_a_state_head_is_unusable() {
        let without = |key: &str| {
            let mut doc = file(vec![]);
            doc.as_object_mut().unwrap().remove(key);
            doc
        };
        let with = |key: &str, value: Value| {
            let mut doc = file(vec![]);
            doc[key] = value;
            doc
        };
        for doc in [
            json!([1, 2]),
            without("version"),
            without("saved_at_ms"),
            without("clean_exit"),
            without("windows"),
            with("version", json!(0)),
            with("version", json!(1.5)),
            with("version", json!(-1)),
            with("version", json!(1e20)),
            with("version", json!(18_446_744_073_709_551_616.0_f64)),
            with("version", json!("1")),
            with("windows", json!({})),
            with("resolved_crash_copy", json!(3)),
        ] {
            assert!(
                matches!(decode(&doc.to_string()), Err(Unusable::NotStateFile(_))),
                "{doc}"
            );
        }
    }

    #[test]
    fn a_bom_is_tolerated() {
        let text = encode(&full_state()).unwrap();
        assert_eq!(
            decode(&format!("\u{feff}{text}")).unwrap(),
            decode(&text).unwrap()
        );
    }

    #[test]
    fn only_invalid_data_read_errors_are_unusable() {
        let over = crate::fsutil::read_capped(&b"0123456789"[..], 4).unwrap_err();
        assert!(matches!(
            unusable_read(&over),
            Some(Unusable::Unreadable(_))
        ));
        let not_utf8 = crate::fsutil::read_capped(&[0xff, 0xfe, 0x00][..], 16).unwrap_err();
        assert!(matches!(
            unusable_read(&not_utf8),
            Some(Unusable::Unreadable(_))
        ));
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Other,
        ] {
            assert_eq!(unusable_read(&io::Error::from(kind)), None, "{kind:?}");
        }
    }

    // ── 쓰기 상한 ──

    #[test]
    fn encode_writes_up_to_the_cap_and_refuses_past_it() {
        let mut state = full_state();
        main_strip(&mut state).tabs[0].name.clear();
        let base = encode(&state).unwrap().len();
        let fill = STATE_READ_CAP as usize - base;

        main_strip(&mut state).tabs[0].name = "x".repeat(fill);
        assert_eq!(encode(&state).unwrap().len() as u64, STATE_READ_CAP);

        main_strip(&mut state).tabs[0].name.push('x');
        assert_eq!(
            encode(&state),
            Err(EncodeError::TooLarge {
                len: STATE_READ_CAP as usize + 1
            })
        );
    }

    /// 분할 `depth` 겹(언제나 `a` 쪽을 다시 나눈다) — 창 하나 · 탭 하나의 파일.
    fn nested_splits(depth: usize) -> StateFile {
        let mut layout = known(SlotContent::Empty);
        for _ in 0..depth {
            layout = PersistedNode::Split {
                id: Uuid::new_v4(),
                dir: SplitDir::LeftRight,
                ratio: 0.5,
                a: Box::new(layout),
                b: Box::new(known(SlotContent::Empty)),
            };
        }
        let only = tab("깊은 탭", layout);
        StateFile {
            version: STATE_VERSION,
            saved_at_ms: 1,
            clean_exit: true,
            resolved_crash_copy: None,
            windows: vec![WindowEntry {
                id: "main".to_string(),
                kind: WindowKind::Main(TabStrip {
                    active_tab: only.id,
                    tabs: vec![only],
                }),
                theme: None,
                bounds: Some(bounds()),
                maximized: false,
            }],
        }
    }

    #[test]
    fn encode_refuses_nesting_that_decode_cannot_read() {
        let deepest_readable = nested_splits(120);
        let text = encode(&deepest_readable).unwrap();
        assert_eq!(decode(&text).unwrap(), (deepest_readable, vec![]));

        let too_deep = nested_splits(121);
        let written = serde_json::to_string_pretty(&too_deep).unwrap();
        assert!(matches!(decode(&written), Err(Unusable::NotJson(_))));
        assert!(matches!(
            encode(&too_deep),
            Err(EncodeError::Unparsable { .. })
        ));
    }
}
