//! The only YAML in the crate: reading preset and settings files ([`read`]), the library's front-matter split and the
//! real [`FrontMatterParser`](engram_dashboard_agent::backend::settings_check::FrontMatterParser) ([`frontmatter`]),
//! and the whole-file preset writer over an order-keeping document tree ([`tree`] · [`write`]).
//!
//! Every parse here goes through [`options()`], so the reader, the writer's tree, its round-trip check and the
//! front-matter parser read the same text the same way. Pure: no disk IO.
// ADR-0336
// ADR-0358
// TRD S25 step-7 §3-4

pub mod frontmatter;
pub mod read;
pub mod tree;
pub mod write;

use engram_dashboard_agent::launch::Reason;
use serde_json::json;

pub use frontmatter::{
    conflict_key, library_skill_name, split_front_matter, FrontMatter, YamlFrontMatter,
};
pub use read::{
    parse_preset, parse_settings, sniff_kind, ListName, PresetDoc, RawField, SettingsDoc, Sniff,
    Unmodeled,
};
pub use tree::Node;
pub use write::{rewrite_lists, FolderItem, PresetLists, WriteError};

/// The parse options of every YAML read in this crate.
///
/// - `strict_booleans` (G49): only `true` / `false` (any case) are booleans; YAML 1.1 `yes` · `no` · `on` · `off` are
///   strings, as a YAML 1.2 reader takes them.
/// - `non_finite_float_policy: PassThrough` (premise 40): `.nan` / `.inf` reach the visitor as floats instead of
///   failing the read, so a non-finite value in a key Engram does not model neither breaks the file nor an edit.
/// - `ignore_binary_tag_for_string`: a `!!binary` scalar is kept as its base64 text (the tag is dropped, the value stays)
///   instead of being decoded — a decoded payload would be rewritten as different text, and one that is not UTF-8
///   would fail the whole file.
///
/// Everything else is serde-saphyr's default: a duplicate key, more than one document and an alias bomb are errors;
/// aliases and merge keys are expanded. On top of this, the preset and settings readers (and the writer through the
/// preset reader) refuse content after an explicit `...` (`content_after_document_end`); the front-matter parser does
/// not.
pub fn options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        strict_booleans: true,
        non_finite_float_policy: serde_saphyr::NonFiniteFloatPolicy::PassThrough,
        ignore_binary_tag_for_string: true,
    }
}

/// Why a file could not be read as a whole. `code` is a reason code (kebab-case); `line` is 1-based in the text that
/// was parsed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ParseError {
    pub code: &'static str,
    pub message: String,
    pub line: Option<u32>,
}

impl ParseError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>, line: Option<u32>) -> Self {
        Self {
            code,
            message: message.into(),
            line,
        }
    }

    /// A serde-saphyr refusal: `multi_document` when the text holds more than one document, `yaml-syntax` otherwise
    /// (syntax, a duplicate key, a value of the wrong type for a typed field, a budget breach).
    pub(crate) fn from_yaml(error: &serde_saphyr::Error, multi_document: &'static str) -> Self {
        let error = error.without_snippet();
        let code = match error {
            serde_saphyr::Error::MultipleDocuments { .. } => multi_document,
            _ => YAML_SYNTAX,
        };
        Self::new(code, error.to_string(), line_of(error))
    }

    /// The block reason of this error.
    pub fn to_reason(&self) -> Reason {
        Reason {
            code: self.code.to_owned(),
            args: json!({ "message": self.message }),
            line: self.line,
        }
    }
}

pub(crate) const YAML_SYNTAX: &str = "yaml-syntax";

pub(crate) fn line_of(error: &serde_saphyr::Error) -> Option<u32> {
    error
        .location()
        .and_then(|location| u32::try_from(location.line()).ok())
        .filter(|&line| line > 0)
}

/// The source text of the node at `location` (`Spanned` positions are byte ranges for a string source); `None` when the
/// position carries no byte range.
///
/// **Judging raw source text — whitelists only.** Wherever this crate decides something from a node's raw text or a
/// raw line, it accepts only an exact, listed spelling and treats every other spelling as "not that": YAML spells one
/// value many ways (quotes, block headers such as `|- # c` or `|2-`, tags, NUL), and a blacklist of the bad ones keeps
/// missing a variant. A spelling outside the list costs a refusal or a raw-text display, never a silent change. Uses:
/// the document-end boundary ([`DOCUMENT_END_FOLLOWERS`]), the writer's out-of-range float guard
/// (`write::changed_by_reading`), and the settings `description` that JSON reads as `null` (`read::json_scalar_node`).
pub(crate) fn source_of<'t>(text: &'t str, location: &serde_saphyr::Location) -> Option<&'t str> {
    let span = location.span();
    let start = usize::try_from(span.byte_offset()?).ok()?;
    let len = usize::try_from(span.byte_len()?).ok()?;
    text.get(start..start.checked_add(len)?)
}

/// The characters that, right after `...`, make it a document end — exactly the string-input set of granit-parser
/// 2.0.0 `next_is_document_end` (`input/str.rs`): space, tab, NUL, `\n`, `\r` (or the end of input).
pub(crate) const DOCUMENT_END_FOLLOWERS: [char; 5] = [' ', '\t', '\0', '\n', '\r'];

/// The 1-based line of the first content after an explicit document end, or `None`.
///
/// serde-saphyr ignores a scan error after an explicit `...` (`de/with_deserializer.rs` — "trailing garbage after an
/// explicit document end marker is ignored"), so such a tail would be read as nothing and deleted by a rewrite. The
/// preset and settings readers ([`read::parse_preset`] · [`read::parse_settings`]) refuse it as a second document,
/// and the writer through `parse_preset`; the front-matter parser does not run this check.
///
/// A document end is a line (YAML line breaks — [`yaml_lines`]) that starts with `...` followed by one of
/// [`DOCUMENT_END_FOLLOWERS`] or the end of the line, as the YAML scanner reads it (whitelist — see [`source_of`]);
/// `...#x` is a plain scalar, not an end. Content = anything other than spaces, tabs and a `#` comment (a NUL
/// included) — on the rest of the `...` line or on a later line.
///
/// Known safe false positives — refusals, never losses: a file whose first line is `...`, two `...` lines in a row
/// (the second counts as content), and `...` followed by a NUL with nothing after it.
pub(crate) fn content_after_document_end(text: &str) -> Option<u32> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let is_content = |rest: &str| {
        let rest = rest.trim_start_matches([' ', '\t']);
        !rest.is_empty() && !rest.starts_with('#')
    };
    let mut ended = false;
    for (line, number) in yaml_lines(text).zip(1u32..) {
        if ended {
            if is_content(line) {
                return Some(number);
            }
        } else if let Some(after) = line.strip_prefix("...") {
            if after.is_empty() || after.starts_with(DOCUMENT_END_FOLLOWERS) {
                if is_content(after) {
                    return Some(number);
                }
                ended = true;
            }
        }
    }
    None
}

/// The lines of `text` split the way the YAML scanner breaks them: `\r\n`, `\n` and a bare `\r` each end one line, so
/// line numbers match serde-saphyr's.
pub(crate) fn yaml_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let current = rest?;
        match current.find(['\r', '\n']) {
            Some(at) => {
                let skip = if current[at..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                rest = Some(&current[at + skip..]);
                Some(&current[..at])
            }
            None => {
                rest = None;
                (!current.is_empty()).then_some(current)
            }
        }
    })
}
