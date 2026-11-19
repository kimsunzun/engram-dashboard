//! The help screen source — builds the screens `/control/help` gives from the body file (`prompts/engram-help.md`).
//!
//! Owns the screen word vocabulary (word → screen · old word aliases), the body file path rule, section parsing, the required-section check, rendering and request body
//! interpretation; the CLI only sends one word and prints the screen (two copies would diverge).
//! It reads the file on every request — do not add a cache: the daemon is long-lived, and a once-read body would not change after editing until relaunch (the purpose
//! of keeping the body outside the binary). Cost: one ~40 KB read per request.
//! A failure is a bounce — no built-in copy, no half screen: an unreadable file or missing required section answers `INTERNAL` with a warn; a half screen hides what
//! was missing and a stale copy hides that the body is broken.
//! It does not look at the caller: the screen is the same for everyone (per-caller filters withdrawn — ADR-0220 decision 4), so this route is not mail and a
//! mail-blocked credential reads it too (`mcp_server::ControlRoute::is_mail`).
//! It does not interpret client text: it only picks a `## <id>` section of the daemon's own distribution file and gives it as is — it touches neither the roster's `help`
//! blob (ADR-0156) nor UI command payloads (ADR-0081). The words `window` · `settings` · `library` are screen names, not UI command names or CLI families; the command names and arguments of those screens exist only in that body's prose.
//! Six screens, all required (`HelpTopic::ALL`): `library` joined with the library, preset and agent-preset commands (ADR-0386).
// ADR-0081
// ADR-0092
// ADR-0156
// ADR-0212
// ADR-0220
// ADR-0285
// ADR-0386

use std::path::PathBuf;

use engram_dashboard_agent::types::{
    CLI_EXE_NAME, CLI_GROUP_AGENT, CLI_GROUP_MAIL, CLI_HELP_SCREEN_KEY, CLI_HELP_TOPIC_KEY,
};
use engram_dashboard_command::ErrorCode;

use super::agent::preview;
use super::ingress::ControlQueryResult;

/// The executable name placeholder of the help template, replaced with `CLI_EXE_NAME` at render time — help is where an agent learns the surface, so a diverging name would not find the command.
const HELP_TOOL_SLOT: &str = "{tool}";

/// The screen body's fixed relative path, in `prompts/`. ADR-0100 ships that folder whole beside the exes and the
/// manifest tripwire (`scripts/build-release.ps1`) catches both a missing and an extra file there, so moving the body
/// to a new folder needs both of those built again for it.
const REL_HELP_FILE: &str = "prompts/engram-help.md";

/// Where an operator or a harness swaps the body (this **daemon process**'s env).
///
/// ★A failure does not fall back to the fixed file★: quietly swapping an explicit choice for another file hides what
///   was read. The request is refused, and the reason carries that path.
const ENV_HELP_FILE: &str = "ENGRAM_HELP_FILE";

/// The head of a section marker line; the whole line must be the marker — substring matching would let one prose line describing the format cut a section (that file's header carries such a sentence).
///
/// A markdown heading, not an HTML comment, on purpose: viewers hide the comment form entirely, so the editor saw no section boundary (7KB read as undivided prose). The header title is level 1 (`# `) and does not match this level-2 pattern.
const SECTION_OPEN: &str = "## ";

/// The section id of the top-level screen; the other five are the screen words as is ([`HelpTopic::section_id`]).
const SECTION_ROOT: &str = "root";

/// The words of the three screens with no family constant (`CLI_GROUP_*`): window, settings and library are called only by full name (`window.list` · `settings.get` · `library.list` · `preset.get`) with no `<family> <verb>` entrance, so family constants would create families the CLI parser does not accept.
const HELP_TOPIC_WINDOW: &str = "window";
const HELP_TOPIC_SETTINGS: &str = "settings";
// ADR-0386
const HELP_TOPIC_LIBRARY: &str = "library";

/// Old screen word → current screen. Accepted only as a request word, not carried in the bounce wording.
///
/// Do not withdraw: agents launched or resumed before the rename (TRD S21-storage §10 F14) hold the old priming (`engram help theme`) and would not find the settings screen.
/// It does not give the body's `theme` section: that is a short pointer left so old binaries requiring `theme` accept the new file; the alias goes straight to settings.
const HELP_TOPIC_ALIASES: &[(&str, HelpTopic)] = &[("theme", HelpTopic::Settings)];

/// Which help screen. One screen = one section = one word (only `Root` has no word) — the old stitched-fragment shape left the editor unable to tell which fragment went on which screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpTopic {
    Root,
    Mail,
    Agent,
    Window,
    Settings,
    Library,
}

impl HelpTopic {
    /// All screens — the required-section list and the word table both come from here, so adding a screen cannot leave out one side.
    const ALL: &'static [HelpTopic] = &[
        HelpTopic::Root,
        HelpTopic::Mail,
        HelpTopic::Agent,
        HelpTopic::Window,
        HelpTopic::Settings,
        HelpTopic::Library,
    ];

    fn section_id(self) -> &'static str {
        match self {
            HelpTopic::Root => SECTION_ROOT,
            HelpTopic::Mail => CLI_GROUP_MAIL,
            HelpTopic::Agent => CLI_GROUP_AGENT,
            HelpTopic::Window => HELP_TOPIC_WINDOW,
            HelpTopic::Settings => HELP_TOPIC_SETTINGS,
            HelpTopic::Library => HELP_TOPIC_LIBRARY,
        }
    }

    /// The word a request carries; no `Root` — a null word position is that screen.
    fn group_word(self) -> Option<&'static str> {
        match self {
            HelpTopic::Root => None,
            other => Some(other.section_id()),
        }
    }

    fn from_group_word(word: &str) -> Option<HelpTopic> {
        HelpTopic::ALL
            .iter()
            .copied()
            .find(|t| t.group_word() == Some(word))
            .or_else(|| {
                HELP_TOPIC_ALIASES
                    .iter()
                    .find(|(alias, _)| *alias == word)
                    .map(|&(_, topic)| topic)
            })
    }
}

/// One sheet of screen body (section id → body). No half load: one missing section throws the table away and bounces the request.
struct HelpText {
    sections: Vec<(String, String)>,
    /// The file read — carried by the missing-section marking and the startup log.
    origin: String,
}

impl HelpText {
    /// Body = from the byte after the marker line to just before the next marker line, as is: indentation · blank lines · trailing newline are screen formatting and untrimmed. Text before the first marker is discarded (the file header).
    /// Only CRLF is folded: `core.autocrlf` makes checked-out line endings differ per machine and the bytes are the contract (the CLI prints exactly what it got).
    fn parse(src: &str, origin: &str) -> Self {
        let mut sections: Vec<(String, String)> = Vec::new();
        let mut current: Option<(String, String)> = None;
        for line in src.split_inclusive('\n') {
            match section_marker(line) {
                Some(id) => {
                    sections.extend(current.take());
                    current = Some((id.to_string(), String::new()));
                }
                None => {
                    if let Some((_, body)) = current.as_mut() {
                        body.push_str(&line.replace("\r\n", "\n"));
                    }
                }
            }
        }
        sections.extend(current.take());
        Self {
            sections,
            origin: origin.to_string(),
        }
    }

    /// A missing section still gives something: the loader picks only complete tables so this never hits, but if that guarantee breaks, leaving the place and id keeps the screen from going silently empty (read as an absent surface).
    fn section(&self, id: &str) -> String {
        self.sections
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| format!("(help section `{id}` missing from {})\n", self.origin))
    }

    /// All the sections the screen assembly requires that this table cannot fill.
    fn missing(&self) -> Vec<String> {
        required_section_ids()
            .into_iter()
            .filter(|id| !self.sections.iter().any(|(k, _)| k == id))
            .collect()
    }
}

/// The section id if it is a marker line: the whole line must be the marker (surrounding whitespace allowed), and the id has no spaces.
fn section_marker(line: &str) -> Option<&str> {
    let inner = line.trim().strip_prefix(SECTION_OPEN)?.trim();
    // A section id is only lowercase words joined by dots — else one ordinary body heading would cut a section.
    let is_id = !inner.is_empty()
        && !inner.contains(char::is_whitespace)
        && inner
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_');
    is_id.then_some(inner)
}

/// All sections the screens require = all screens. One screen is one section, so there is no derivation.
fn required_section_ids() -> Vec<String> {
    HelpTopic::ALL
        .iter()
        .map(|t| t.section_id().to_string())
        .collect()
}

/// One screen = one section + executable name substitution.
fn render_help_from(text: &HelpText, topic: HelpTopic) -> String {
    text.section(topic.section_id())
        .replace(HELP_TOOL_SLOT, CLI_EXE_NAME)
}

/// The source of help screens — where to find the body file; the assembly builds it and hands it to the server (`mcp_server::start_mcp_server_with_help`).
///
/// ★Path shape★: the fixed relative path is joined to base; a non-empty `ENGRAM_HELP_FILE` comes first and does not
///   fall back to the fixed path when it fails; only an absolute path is read. ★cwd is not used★ — a daemon launched
///   through WMI has System32 as its cwd. This process reads the content itself.
/// It takes base so tests can hand over a known-body folder regardless of the test exe location · `CARGO_TARGET_DIR`; hence it is not built from the install location inside the server.
// ADR-0092
// ADR-0285
pub struct HelpSource {
    anchor: Anchor,
}

enum Anchor {
    /// The folder to attach the fixed relative path to.
    Base(PathBuf),
    /// The production constructor could not get the install location — only an absolute-path override is read.
    NoInstallRoot,
    /// The server version that does not call help (`start_mcp_server_without_help`) — bounces every request regardless of env.
    NotConfigured,
}

impl HelpSource {
    pub fn new(base: PathBuf) -> Self {
        Self {
            anchor: Anchor::Base(base),
        }
    }

    /// The production constructor — anchored at the install location (`data_dir::find_install_root`). In a release that
    /// is the exe folder, and `prompts/` ships beside it (ADR-0100).
    pub fn from_install_root() -> Self {
        match crate::data_dir::find_install_root() {
            Some(root) => Self::new(root),
            None => Self {
                anchor: Anchor::NoInstallRoot,
            },
        }
    }

    pub(super) fn not_configured() -> Self {
        Self {
            anchor: Anchor::NotConfigured,
        }
    }

    /// Answers one request — success `{"screen": …}` or a bounce envelope, always one value. Blocking (file read).
    ///
    /// The word decision comes before the file read: the vocabulary is code, so an unknown word is `INVALID_ARGUMENT` even with a broken body — what the caller fixes (word) and what the owner fixes (body) stay separate codes.
    pub fn answer(&self, body: &[u8]) -> ControlQueryResult {
        let topic = match parse_request(body) {
            Ok(topic) => topic,
            Err(rejected) => return rejected,
        };
        match self.load() {
            Ok(text) => {
                let mut screen = serde_json::Map::new();
                screen.insert(
                    CLI_HELP_SCREEN_KEY.to_string(),
                    serde_json::Value::String(render_help_from(&text, topic)),
                );
                ControlQueryResult::Ok(serde_json::Value::Object(screen))
            }
            Err(e) => {
                e.warn("could not give the help screen — bouncing with INTERNAL");
                ControlQueryResult::Error {
                    code: ErrorCode::Internal.as_str(),
                    hint: format!("help text unavailable — {e}; report this to the owner"),
                    args: None,
                }
            }
        }
    }

    /// Once at startup — records the body resolved now (diagnosis, not decision; requests reread). Normal is info (invisible at the default warn level); unusable is warn so it remains even in a release daemon `RUST_LOG` does not reach.
    pub(super) fn log_startup(&self) {
        match self.load() {
            Ok(text) => tracing::info!(
                path = %text.origin,
                "help body source checked — /control/help answers with this file"
            ),
            Err(e) => e.warn("help body source unusable — /control/help answers with INTERNAL"),
        }
    }

    fn path(&self) -> Result<PathBuf, Unavailable> {
        let base = match &self.anchor {
            Anchor::Base(base) => Some(base.as_path()),
            Anchor::NoInstallRoot => None,
            Anchor::NotConfigured => {
                return Err(Unavailable::bare(
                    "help source not configured on this server",
                ))
            }
        };
        let chosen = std::env::var_os(ENV_HELP_FILE)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(REL_HELP_FILE));
        if chosen.is_absolute() {
            return Ok(chosen);
        }
        let Some(base) = base else {
            return Err(Unavailable::bare(format!(
                "the install folder could not be found, so {} has no absolute path",
                chosen.display()
            )));
        };
        let joined = base.join(&chosen);
        if joined.is_absolute() {
            Ok(joined)
        } else {
            Err(Unavailable::at(joined, "not an absolute path"))
        }
    }

    fn load(&self) -> Result<HelpText, Unavailable> {
        let path = self.path()?;
        let src = std::fs::read_to_string(&path)
            .map_err(|e| Unavailable::at(path.clone(), e.to_string()))?;
        let text = HelpText::parse(&src, &path.display().to_string());
        let missing = text.missing();
        if missing.is_empty() {
            Ok(text)
        } else {
            Err(Unavailable::at(
                path,
                format!("missing sections: {}", missing.join(", ")),
            ))
        }
    }
}

/// The reason the body could not be used — the bounce wording and the warn carry the same value.
struct Unavailable {
    /// The file it tried to read. Absent if it stopped before building the path.
    path: Option<PathBuf>,
    reason: String,
}

impl Unavailable {
    fn bare(reason: impl Into<String>) -> Self {
        Self {
            path: None,
            reason: reason.into(),
        }
    }

    fn at(path: PathBuf, reason: impl Into<String>) -> Self {
        Self {
            path: Some(path),
            reason: reason.into(),
        }
    }

    fn warn(&self, what: &str) {
        match &self.path {
            Some(path) => tracing::warn!(path = %path.display(), reason = %self.reason, "{what}"),
            None => tracing::warn!(reason = %self.reason, "{what}"),
        }
    }
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{}: {}", path.display(), self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

/// Request body → screen; a bounce is an envelope to return as is.
///
/// `topic` is required and other keys are bounced: an empty body read as the index would hide a CLI key-name mismatch, and unknown keys would silently ignore a misspelling. ADR-0157's unknown-field
/// tolerance is for wiring across a build boundary; the CLI and daemon ship in one build (ADR-0100), so this route has none. Decided by [`HelpRequest`] deserialization.
// ADR-0157
// ADR-0285
fn parse_request(body: &[u8]) -> Result<HelpTopic, ControlQueryResult> {
    let HelpRequest { topic } = serde_json::from_slice(body).map_err(|e| {
        malformed_request(&format!("is malformed ({})", preview(&e.to_string())), body)
    })?;
    let Some(word) = topic else {
        return Ok(HelpTopic::Root);
    };
    HelpTopic::from_group_word(&word).ok_or_else(|| ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "unknown help topic: {} — run `{CLI_EXE_NAME} help` to list groups, or `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`",
            preview(&word)
        ),
        args: None,
    })
}

/// The bounce when the body failed the contract — carries the material to fix (the expected shape) and the head of the received body.
fn malformed_request(what: &str, body: &[u8]) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "the help request body {what} — it must be a JSON object like {{\"{CLI_HELP_TOPIC_KEY}\":null}} for the list of groups or {{\"{CLI_HELP_TOPIC_KEY}\":\"{CLI_GROUP_MAIL}\"}} for one group; got: {}",
            preview(&String::from_utf8_lossy(body))
        ),
        args: None,
    }
}

/// The help request body — a JSON object with only the key [`CLI_HELP_TOPIC_KEY`]. `topic: None` = index.
struct HelpRequest {
    topic: Option<String>,
}

impl<'de> serde::Deserialize<'de> for HelpRequest {
    /// A hand-written visitor — do not revert to `#[derive(Deserialize)]` or `serde_json::Value`:
    ///   - `Value` folds a repeated key into the later value, so `{"topic":"mail","topic":"agent"}` answers one screen; here a duplicate is a `duplicate field` bounce.
    ///   - The derive reads a JSON array as a field sequence, so `["mail"]` becomes a screen (the trap `catalog::CallRequest` stepped on); only `deserialize_map` is accepted.
    ///   - The derive fills an `Option` with `None` when the key is missing, so a misspelled key would answer the index.
    // ADR-0285
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        const FIELDS: &[&str] = &[CLI_HELP_TOPIC_KEY];
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = HelpRequest;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    f,
                    "a JSON object with exactly one key `{CLI_HELP_TOPIC_KEY}`"
                )
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<HelpRequest, M::Error> {
                let mut topic: Option<Option<String>> = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key != CLI_HELP_TOPIC_KEY {
                        return Err(serde::de::Error::unknown_field(&key, FIELDS));
                    }
                    if topic.is_some() {
                        return Err(serde::de::Error::duplicate_field(CLI_HELP_TOPIC_KEY));
                    }
                    topic = Some(map.next_value()?);
                }
                match topic {
                    Some(topic) => Ok(HelpRequest { topic }),
                    None => Err(serde::de::Error::missing_field(CLI_HELP_TOPIC_KEY)),
                }
            }
        }

        de.deserialize_map(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_agent::types::{CLI_AGENT_FLAGS, CLI_AGENT_VERBS};
    use std::sync::{Mutex, MutexGuard};

    /// `ENGRAM_HELP_FILE` is process-global: every test reading through the source holds this lock with the variable cleared, or it would measure the wrong file.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> MutexGuard<'static, ()> {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var(ENV_HELP_FILE);
        guard
    }

    /// Tests run inside the compile-time source tree, so MANIFEST_DIR can be trusted.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root")
            .to_path_buf()
    }

    /// The verbatim distributed body file — the very file the release ships in `prompts/` next to the exe.
    fn shipped_file() -> String {
        std::fs::read_to_string(repo_root().join(REL_HELP_FILE))
            .expect("the body file must be at REL_HELP_FILE")
    }

    fn shipped_text() -> HelpText {
        HelpText::parse(&shipped_file(), "shipped")
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-help-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        dir
    }

    /// A folder holding only `<base>\prompts\engram-help.md` — the shape the release assembly puts next to the exe.
    fn release_shaped(tag: &str, body: &str) -> PathBuf {
        let base = temp_dir(tag);
        let file = base.join(REL_HELP_FILE);
        std::fs::create_dir_all(file.parent().expect("prompts folder")).expect("prompts folder");
        std::fs::write(&file, body).expect("write body");
        base
    }

    fn request(word: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ CLI_HELP_TOPIC_KEY: word })).expect("request body")
    }

    fn screen_of(source: &HelpSource, word: Option<&str>) -> String {
        match source.answer(&request(word)) {
            ControlQueryResult::Ok(v) => v[CLI_HELP_SCREEN_KEY]
                .as_str()
                .unwrap_or_else(|| {
                    panic!("the success envelope must have a screen string ({word:?}): {v}")
                })
                .to_string(),
            ControlQueryResult::Error { code, hint, .. } => {
                panic!("must be a screen ({word:?}): {code} {hint}")
            }
        }
    }

    fn rejection_of(source: &HelpSource, body: &[u8]) -> (&'static str, String) {
        match source.answer(body) {
            ControlQueryResult::Error { code, hint, .. } => (code, hint),
            ControlQueryResult::Ok(v) => panic!("must be a bounce: {v}"),
        }
    }

    // ── the distributed body file ─────────────────────────────────────────────────────────────

    /// If the distributed file is incomplete, help is bounced entirely — there is no copy, so this file is every screen's only home. Section ids derive from topics, so a screen added without a body section turns this red.
    #[test]
    fn the_shipped_help_file_carries_every_section_the_screens_need() {
        let text = shipped_text();
        assert!(
            text.missing().is_empty(),
            "sections missing from the distributed body: {:?}",
            text.missing()
        );
    }

    /// Every screen stands from the one distributed file — each word drawn through the real request path.
    #[test]
    fn every_screen_renders_from_the_shipped_help_file() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let words = std::iter::once(None)
            .chain(
                HelpTopic::ALL
                    .iter()
                    .filter_map(|t| t.group_word().map(Some)),
            )
            .chain(HELP_TOPIC_ALIASES.iter().map(|(alias, _)| Some(*alias)));
        for word in words {
            let screen = screen_of(&source, word);
            assert!(!screen.trim().is_empty(), "empty screen ({word:?})");
            assert!(
                !screen.contains(HELP_TOOL_SLOT),
                "unsubstituted placeholder ({word:?}): {screen}"
            );
            assert!(
                screen.contains(CLI_EXE_NAME),
                "no executable name ({word:?}): {screen}"
            );
        }
    }

    /// All sections the pre-rename binary required remain in the body file: if one is missing, that binary rejects the whole new file and gives its built-in copy (the old guidance to fix `theme`). This source ignores those as unknown sections, and `theme` aliases to settings.
    #[test]
    fn the_help_file_still_satisfies_the_pre_rename_binary_and_this_one_ignores_the_extra() {
        let text = shipped_text();
        for id in ["root", "mail", "agent", "window", "theme"] {
            assert!(
                text.sections.iter().any(|(k, _)| k == id),
                "section `{id}` that the pre-rename binary requires is missing"
            );
        }
        assert!(
            text.missing().is_empty(),
            "extra sections must not break the load"
        );
        let compat = text.section("theme");
        let settings = render_help_from(&text, HelpTopic::Settings);
        assert!(
            settings.starts_with(&format!("{CLI_EXE_NAME} {HELP_TOPIC_SETTINGS} ")),
            "the alias target must be the settings section: {settings}"
        );
        assert_ne!(
            settings,
            compat.replace(HELP_TOOL_SLOT, CLI_EXE_NAME),
            "`help theme` must not give the pointer section meant for old binaries"
        );
    }

    // ── reply contract · priming bridge ─────────────────────────────────────────────────────

    /// A pin moved over (`control/priming.rs::production_priming_files_teach_the_reply_contract`): the envelope grammar came down to this screen (priming shrank to a pointer), so the file that owns the surface guards it.
    ///
    /// At stake: the daemon sends `type="request"` envelopes and on overdue fires a `<notice>` at the sender, but the reply itself is LLM compliance (soft — ADR-0103 decisions 2/3). If this screen does not teach the reply rule,
    /// strict matching (the received id required) structurally cannot get replies.
    /// The spelling is checked as this entrance's own (ADR-0126 decision 1): the reply-target word belongs to the arrived envelope (`in-reply-to`), while the tool-argument notation (snake_case `reply_to`·`reply_by`) is carried by
    /// `control/mcp_server.rs::the_send_message_entry_teaches_its_own_call`. Pulling one spelling into the other screen would revive the abolished bypass teaching.
    // ADR-0103
    // ADR-0126
    #[test]
    fn the_mail_screen_teaches_the_reply_contract() {
        let screen = render_help_from(&shipped_text(), HelpTopic::Mail);
        assert!(
            screen.contains("type=\"request\""),
            "must teach to recognize a request envelope: {screen}"
        );
        assert!(
            screen.contains("<notice>"),
            "must teach that a notice is not a reply target (a daemon-only tag, no from): {screen}"
        );
        assert!(
            screen.contains("in-reply-to"),
            "must teach to recognize an answer that came with the received id: {screen}"
        );
        assert!(
            screen.contains("reply-by"),
            "must teach what the deadline is (the sender's deadline, not the recipient's): {screen}"
        );
        // A pin moved over from `the_send_message_entry_teaches_its_own_call` in `mcp_server.rs` — do not delete: "do not read `pending` as a single cause; query" (ADR-0211 decision 2)
        //   used to be in the tool description; with the out-of-argument contract stripped from it, this screen is its only home.
        assert!(
            screen.contains("pending") && screen.contains("eg_messages"),
            "must teach not to conclude the other side's state from pending but to query with eg_messages: {screen}"
        );
    }

    /// The priming's pointer and this screen point at the same names: priming shrank to the line "type `engram help <family>`", the only bridge where an agent meets the surface. Changing a screen
    /// word or removing that line would leave the agent never finding the surface with no error (each side is fine in its own file); this catches that. Whether the CLI parses that line as a help request is the CLI-side test.
    ///
    /// An accepted word is not enough: an empty screen or a leftover placeholder (`{tool}`) equals nothing at the pointed place, so it sends a real request with that word and checks the screen stands.
    // ADR-0092
    #[test]
    fn the_priming_pointer_names_help_entries_that_actually_render() {
        let _env = env_guard();
        let priming = std::fs::read_to_string(repo_root().join("builtin/instructions/priming.md"))
            .expect("the priming file exists");
        let pointer = format!("{CLI_EXE_NAME} help");
        assert!(
            priming.contains(&pointer),
            "priming must point at `{pointer}` (without this line the agent cannot find the surface)"
        );

        let source = HelpSource::new(repo_root());
        for topic in HelpTopic::ALL {
            let word = topic.group_word();
            if let Some(word) = word {
                assert!(
                    priming.contains(&format!("{pointer} {word}")),
                    "priming must point at `{pointer} {word}`"
                );
            }
            let screen = screen_of(&source, word);
            assert!(!screen.trim().is_empty(), "the screen is empty ({topic:?})");
            assert!(
                !screen.contains(HELP_TOOL_SLOT),
                "no unsubstituted placeholder may remain ({topic:?}): {screen}"
            );
        }

        // The opposite direction — every word priming points at is a real screen: the check above only checks each screen has its line, so a priming typo (`help mailx`) would stay green while an agent typing it gets a bounce.
        let named: Vec<&str> = priming
            .match_indices(&format!("{pointer} "))
            .filter_map(|(at, p)| priming[at + p.len()..].split_whitespace().next())
            .collect();
        assert!(!named.is_empty(), "priming has no `{pointer} <word>` line");
        for word in named {
            assert!(
                HelpTopic::from_group_word(word).is_some(),
                "the `{pointer} {word}` priming points at is not a screen word"
            );
            let screen = screen_of(&source, Some(word));
            assert!(
                !screen.trim().is_empty() && !screen.contains(HELP_TOOL_SLOT),
                "the `{pointer} {word}` screen priming points at does not stand: {screen}"
            );
        }
    }

    // ── word → screen ─────────────────────────────────────────────────────────────────

    /// The family list screen and the agent screen teach the surface as is — a nonexistent verb makes the LLM try a nonexistent command (the unresolved part of ADR-0122).
    #[test]
    fn the_agent_screen_documents_every_verb_and_flag() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let root = screen_of(&source, None);
        let mail = screen_of(&source, Some(CLI_GROUP_MAIL));
        let agent = screen_of(&source, Some(CLI_GROUP_AGENT));
        assert!(
            root.contains(CLI_GROUP_AGENT),
            "agent in the family list: {root}"
        );
        assert!(
            root.contains(CLI_GROUP_MAIL),
            "mail in the family list: {root}"
        );
        for verb in CLI_AGENT_VERBS {
            assert!(agent.contains(verb), "verb {verb} in help: {agent}");
        }
        for flag in CLI_AGENT_FLAGS {
            assert!(agent.contains(flag), "{flag} in help: {agent}");
        }
        for absent in ["kill", " rm ", "delete"] {
            assert!(
                !agent.contains(absent),
                "a verb not on the surface is in help ({absent}): {agent}"
            );
        }
        for text in [&root, &mail, &agent] {
            assert!(
                !text.contains(HELP_TOOL_SLOT),
                "no unsubstituted placeholder may remain: {text}"
            );
            assert!(
                text.contains(CLI_EXE_NAME),
                "the executable name must come from the constant"
            );
        }
    }

    /// The library screen teaches every library, preset and agent-preset command by its full name (full names only —
    /// the CLI has no `library` family), and the rules no single command summary carries: the file formats with a
    /// `description:`, the description rule, the folder entry form, the whole-file rewrite (ADR-0358), the two edit
    /// guards, how to read a refusal, and the check before a creating call is repeated. It never says a missing item
    /// blocks a launch — that stops being true when launches skip missing items (ADR-0374). The agent screen names the
    /// two new verbs and `--presets` and points at this screen. A command added to the library port without a line here
    /// turns this red.
    // ADR-0358
    // ADR-0374
    // ADR-0386
    #[test]
    fn the_library_screen_teaches_every_library_command_and_the_edit_rules() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let root = screen_of(&source, None);
        let library = screen_of(&source, Some(HELP_TOPIC_LIBRARY));
        let agent = screen_of(&source, Some(CLI_GROUP_AGENT));
        assert!(
            root.contains(&format!("{CLI_EXE_NAME} help {HELP_TOPIC_LIBRARY} ")),
            "the root screen lists the library screen: {root}"
        );

        let agent_presets = [
            "agent.presets",
            "agent.attachPreset",
            "agent.detachPreset",
            "agent.reorderPresets",
        ];
        let names: Vec<&str> = engram_dashboard_agent::commands::COMMAND_SPECS
            .iter()
            .map(|spec| spec.name)
            .filter(|name| {
                name.starts_with("library.")
                    || name.starts_with("preset.")
                    || agent_presets.contains(name)
            })
            .collect();
        assert_eq!(
            names.len(),
            21,
            "9 library, 8 preset and 4 agent-preset commands are declared: {names:?}"
        );
        for name in names {
            // A whole word — `agent.presets` must not pass on the strength of `agent.presetsX`.
            let line = format!("{CLI_EXE_NAME} {name}");
            assert!(
                library.contains(&format!("{line} ")) || library.contains(&format!("{line}\n")),
                "{name} by its full name on the library screen: {library}"
            );
        }

        for taught in [
            "kind: preset",
            "description:",
            "folders:",
            "Write a one-line description when you create or edit a settings or preset file.",
            "An edit through Engram rewrites the whole preset file",
            "root_generation",
            "root-generation-stale",
            "base_rev",
            "rev-mismatch",
            "args.code",
            "clear the cause, then repeat the same call",
            "workspace-not-found",
            "check agent.list before calling again",
        ] {
            assert!(
                library.contains(taught),
                "the library screen teaches `{taught}`: {library}"
            );
        }
        assert!(
            !library.contains("block"),
            "the library screen must not tie a missing item to a blocked launch: {library}"
        );

        let pointer = format!("{CLI_EXE_NAME} help {HELP_TOPIC_LIBRARY}");
        for taught in [
            "agent.new --backend",
            "--presets",
            "agent.setWorkspace --target",
            "agent.duplicate --target",
            "check agent.list before calling again",
            pointer.as_str(),
        ] {
            assert!(
                agent.contains(taught),
                "the agent screen teaches `{taught}`: {agent}"
            );
        }
    }

    /// Every `--flag` on a command line of the library screen (`  engram <name> …`) is a declared argument of that
    /// command — a flag the LLM learns here and the declaration does not take is refused by the CLI before it is sent.
    // ADR-0386
    #[test]
    fn every_flag_on_the_library_screen_is_a_declared_argument() {
        let _env = env_guard();
        let library = screen_of(&HelpSource::new(repo_root()), Some(HELP_TOPIC_LIBRARY));
        let lead = format!("  {CLI_EXE_NAME} ");
        let mut checked = 0;
        for line in library.lines().filter(|l| l.starts_with(&lead)) {
            let mut tokens = line[lead.len()..].split_whitespace();
            let name = tokens.next().expect("a command name after the executable");
            let spec = engram_dashboard_agent::commands::COMMAND_SPECS
                .iter()
                .find(|spec| spec.name == name)
                .unwrap_or_else(|| {
                    panic!("`{name}` on the library screen is not declared: {line}")
                });
            let schema: serde_json::Value =
                serde_json::from_str(spec.args_schema).expect("an args schema");
            for flag in tokens.filter_map(|t| {
                t.trim_start_matches(['(', '['])
                    .strip_prefix("--")
                    .map(|f| f.trim_end_matches([')', ']', '|']))
            }) {
                assert!(
                    schema["properties"].get(flag).is_some(),
                    "--{flag} is not an argument of {name}: {line}"
                );
                checked += 1;
            }
        }
        assert!(
            checked > 0,
            "no flag found on the library screen: {library}"
        );
    }

    /// The five screen words each reach their own screen — a word wired to another screen still shows a success envelope, so it checks they differ from each other and the head is its own family.
    #[test]
    fn each_group_word_renders_its_own_screen() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let mut seen: Vec<(&str, String)> = Vec::new();
        for topic in HelpTopic::ALL {
            let Some(word) = topic.group_word() else {
                continue;
            };
            let screen = screen_of(&source, Some(word));
            assert!(
                screen.starts_with(&format!("{CLI_EXE_NAME} {word} ")),
                "the screen head must be its own family ({word}): {screen}"
            );
            seen.push((word, screen));
        }
        assert_eq!(seen.len(), 5, "the screen words are five: {seen:?}");
        let root = screen_of(&source, None);
        for (word, screen) in &seen {
            assert_ne!(*screen, root, "{word} is the same as the top-level screen");
            assert_eq!(
                seen.iter().filter(|(_, s)| s == screen).count(),
                1,
                "another screen gives the same body as {word}"
            );
        }
    }

    #[test]
    fn the_old_theme_word_reaches_the_settings_screen() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        assert_eq!(
            screen_of(&source, Some("theme")),
            screen_of(&source, Some(HELP_TOPIC_SETTINGS)),
            "the old word `theme` must give the settings screen"
        );
        // If an alias equaled a family word, the family side would catch it first and the alias would be a dead line.
        for (alias, _) in HELP_TOPIC_ALIASES {
            assert!(
                HelpTopic::ALL.iter().all(|t| t.group_word() != Some(alias)),
                "alias `{alias}` overlaps a family word"
            );
        }
    }

    // ── path rule · failure = bounce ──────────────────────────────────────────────────────

    /// The fixed path resolution is an absolute path attached to base — neither cwd nor outside base.
    #[test]
    fn the_fixed_help_path_resolves_absolute_to_a_real_file() {
        let _env = env_guard();
        let p = HelpSource::new(repo_root())
            .path()
            .unwrap_or_else(|e| panic!("with an absolute base the path is produced: {e}"));
        assert!(p.is_absolute(), "must be an absolute path: {p:?}");
        assert!(
            p.ends_with("prompts/engram-help.md") || p.ends_with("prompts\\engram-help.md"),
            "must end with the fixed relative path: {p:?}"
        );
        assert!(
            p.is_file(),
            "must point at a real file in the source tree: {p:?}"
        );
    }

    /// No half load: a file missing one section is thrown away whole and bounced with `INTERNAL` — a half screen is worse than a stale one; the missing part just looks like an absent surface.
    #[test]
    fn a_help_file_missing_one_section_is_rejected_whole() {
        let _env = env_guard();
        let full = shipped_file().replace("\r\n", "\n");
        let marker = format!("{SECTION_OPEN}{HELP_TOPIC_SETTINGS}");
        assert!(
            full.contains(&marker),
            "the marker line notation changed: {marker}"
        );
        let crippled = full.replace(&marker, "<!-- just a comment -->");
        assert_eq!(
            HelpText::parse(&crippled, "crippled").missing(),
            vec![HELP_TOPIC_SETTINGS.to_string()],
            "must name the missing section exactly"
        );

        let base = release_shaped("crippled", &crippled);
        // Only settings is missing, but other screens are bounced too — the table is thrown away whole.
        let (code, hint) = rejection_of(&HelpSource::new(base.clone()), &request(Some("mail")));
        assert_eq!(code, ErrorCode::Internal.as_str());
        assert!(
            hint.starts_with("help text unavailable — ")
                && hint.ends_with("; report this to the owner")
                && hint.contains(HELP_TOPIC_SETTINGS),
            "must carry the missing section and say to tell the owner: {hint}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The last floor — even an entirely empty table does not give an empty screen (read as "no such surface"); it leaves the name of what is missing. The loader picks only complete tables so production never hits it; pinned for the day that guarantee breaks.
    #[test]
    fn a_screen_never_renders_empty_even_with_no_sections_at_all() {
        let empty = HelpText::parse("", "nothing at all");
        assert_eq!(empty.missing().len(), required_section_ids().len());
        for topic in HelpTopic::ALL {
            let screen = render_help_from(&empty, *topic);
            assert!(!screen.trim().is_empty(), "empty screen ({topic:?})");
            assert!(
                screen.contains("missing from") && screen.contains("nothing at all"),
                "must leave what is missing from where ({topic:?}): {screen}"
            );
        }
    }

    /// The marker must be the whole line: a prose line describing the format would otherwise cut a section — the body header carries such a sentence, before the first marker, so it must be discarded.
    #[test]
    fn a_section_marker_must_be_a_whole_line() {
        let src = format!(
            "header: the marker has the form `{SECTION_OPEN}<id>`.\n\
             {SECTION_OPEN}one\n\
             body one\n\
             here too {SECTION_OPEN}two is only described\n"
        );
        let text = HelpText::parse(&src, "fixture");
        assert_eq!(
            text.sections.len(),
            1,
            "a notation inside prose cut a section: {:?}",
            text.sections
        );
        assert_eq!(
            text.section("one"),
            format!("body one\nhere too {SECTION_OPEN}two is only described\n")
        );
    }

    /// The body keeps whitespace as is (indentation · blank lines · trailing newline are formatting) — only CRLF is folded (`core.autocrlf` varies checked-out line endings; the bytes are the contract).
    #[test]
    fn section_bodies_keep_their_whitespace_but_fold_crlf() {
        let text = HelpText::parse("## s\r\n\r\n  indented\r\ntail\r\n", "f");
        assert_eq!(text.section("s"), "\n  indented\ntail\n");
        let no_trailing_newline = HelpText::parse("## s\nlast", "f");
        assert_eq!(no_trailing_newline.section("s"), "last");
    }

    /// ★The env override beats the fixed file, and its failure is a **refusal** — no fallback to the fixed file★
    ///   (quietly swapping an explicit choice for another file hides what was read). base holds a sound fixed file, so a
    ///   fallback would render a screen and turn this assertion red.
    /// It reads on every request: after changing only the file, the next request must give the changed text — a cache would serve the old text until relaunch.
    // ADR-0092
    #[test]
    fn the_env_override_wins_and_its_failure_is_a_rejection_not_a_fallback() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let dir = temp_dir("override");
        let custom = dir.join("custom-help.md");
        let landmark = "what you can do on this team";
        let shipped = shipped_file();
        assert!(
            shipped.contains(landmark),
            "the top-level screen's landmark changed"
        );

        std::fs::write(
            &custom,
            shipped.replace(landmark, &format!("{landmark}(custom)")),
        )
        .unwrap();
        std::env::set_var(ENV_HELP_FILE, &custom);
        assert!(
            screen_of(&source, None).contains("(custom)"),
            "the override file must win"
        );

        std::fs::write(
            &custom,
            shipped.replace(landmark, &format!("{landmark}(edited)")),
        )
        .unwrap();
        let edited = screen_of(&source, None);
        assert!(
            edited.contains("(edited)") && !edited.contains("(custom)"),
            "the next request after editing the file must give the edited text (reads on every request): {edited}"
        );

        let ghost = dir.join("does-not-exist.md");
        std::env::set_var(ENV_HELP_FILE, &ghost);
        let (code, hint) = rejection_of(&source, &request(None));
        assert_eq!(
            code,
            ErrorCode::Internal.as_str(),
            "an override failure goes to a bounce, not the fixed file"
        );
        assert!(
            hint.contains("does-not-exist.md"),
            "the bounce must carry the override path it tried to read: {hint}"
        );

        std::env::remove_var(ENV_HELP_FILE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── request body · release shape ──────────────────────────────────────────────────────

    /// The whole body contract: `topic` required (null = index); other keys · types · non-object bodies bounced. An unknown word is bounced before reading the body, so it is `INVALID_ARGUMENT` even on a source with no body, where a known word gives `INTERNAL`.
    #[test]
    fn the_request_body_must_name_one_topic_and_nothing_else() {
        assert_eq!(parse_request(&request(None)).ok(), Some(HelpTopic::Root));
        assert_eq!(
            parse_request(&request(Some(CLI_GROUP_MAIL))).ok(),
            Some(HelpTopic::Mail)
        );
        assert_eq!(
            parse_request(&request(Some("theme"))).ok(),
            Some(HelpTopic::Settings)
        );
        let invalid = ErrorCode::InvalidArgument.as_str();
        for body in [
            "{}".to_string(),
            format!("{{\"{CLI_HELP_TOPIC_KEY}\":\"{CLI_GROUP_MAIL}\",\"extra\":1}}"),
            "{\"topc\":\"mail\"}".to_string(),
            format!("{{\"{CLI_HELP_TOPIC_KEY}\":3}}"),
            format!("{{\"{CLI_HELP_TOPIC_KEY}\":[\"{CLI_GROUP_MAIL}\"]}}"),
            "[]".to_string(),
            format!("[\"{CLI_GROUP_MAIL}\"]"),
            "\"mail\"".to_string(),
            "not json".to_string(),
            String::new(),
        ] {
            match parse_request(body.as_bytes()) {
                Err(ControlQueryResult::Error { code, hint, .. }) => {
                    assert_eq!(code, invalid, "{body:?}: {hint}");
                    assert!(
                        hint.contains(CLI_HELP_TOPIC_KEY),
                        "must carry the shape to fix ({body:?}): {hint}"
                    );
                }
                other => panic!("must be a bounce ({body:?}): {other:?}"),
            }
        }

        let unconfigured = HelpSource::not_configured();
        let (code, hint) = rejection_of(&unconfigured, &request(Some("wat")));
        assert_eq!(
            code, invalid,
            "an unknown word is bounced before reading the body"
        );
        assert_eq!(
            hint,
            format!(
                "unknown help topic: wat — run `{CLI_EXE_NAME} help` to list groups, or `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`"
            )
        );
        let (code, hint) = rejection_of(&unconfigured, &request(Some(CLI_GROUP_MAIL)));
        assert_eq!(code, ErrorCode::Internal.as_str());
        assert!(
            hint.contains("help source not configured on this server"),
            "{hint}"
        );
    }

    /// A key that comes twice is bounced, not folded into either side — folding would let this daemon decide alone what the body asked; even equal values (two nulls) bounce, since bouncing must not depend on the value.
    #[test]
    fn a_repeated_topic_key_is_rejected_not_collapsed() {
        for body in [
            r#"{"topic":"mail","topic":"agent"}"#,
            r#"{"topic":null,"topic":null}"#,
            r#"{"topic":"mail","topic":"mail"}"#,
        ] {
            match parse_request(body.as_bytes()) {
                Err(ControlQueryResult::Error { code, hint, .. }) => {
                    assert_eq!(code, ErrorCode::InvalidArgument.as_str(), "{body}: {hint}");
                    assert!(
                        hint.contains("duplicate field") && hint.contains(CLI_HELP_TOPIC_KEY),
                        "must say it is a duplicate key ({body}): {hint}"
                    );
                }
                other => panic!("must be a bounce ({body}): {other:?}"),
            }
        }
    }

    /// Screens stand in a release-shaped folder: the release puts the exe and `prompts\` in one folder, so a source with that base gives screens without a checkout; a folder without `prompts\` (e.g. a `CARGO_TARGET_DIR` exe folder outside the checkout) is `INTERNAL`.
    #[test]
    fn a_release_shaped_folder_renders_and_one_without_prompts_is_internal() {
        let _env = env_guard();
        let release = release_shaped("release", &shipped_file());
        let screen = screen_of(&HelpSource::new(release.clone()), Some(CLI_GROUP_MAIL));
        assert_eq!(
            screen,
            render_help_from(&shipped_text(), HelpTopic::Mail),
            "the screen of a release-shaped folder must be the same as the screen of the distributed body"
        );

        let bare = temp_dir("bare");
        let (code, hint) = rejection_of(&HelpSource::new(bare.clone()), &request(None));
        assert_eq!(code, ErrorCode::Internal.as_str(), "{hint}");
        assert!(
            hint.contains("engram-help.md"),
            "must carry the path it tried to read: {hint}"
        );

        let _ = std::fs::remove_dir_all(&release);
        let _ = std::fs::remove_dir_all(&bare);
    }
}
