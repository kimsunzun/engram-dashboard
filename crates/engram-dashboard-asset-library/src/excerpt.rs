//! The one-line excerpt of an instruction file shown in the list (R15): the text of its first paragraph with
//! whitespace collapsed, at most [`EXCERPT_CHARS`] characters. Front matter, code blocks, tables and HTML are not
//! text for this purpose. Display only — nothing decides anything from it.
// TRD S25 step-7 §3-5 · B16

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::format::split_front_matter;

/// The excerpt's length cap in characters (`char`s); the client shortens it further for display.
pub const EXCERPT_CHARS: usize = 200;

/// The excerpt of `text` (a whole `.md` file). Empty when no paragraph holds text.
///
/// Front matter is cut with the library's split ([`split_front_matter`]) after a leading BOM; the Markdown walk also
/// skips a YAML metadata block, so a front matter the split does not take is still not text.
pub fn excerpt(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let body = split_front_matter(text).map_or(text, |front| front.body);
    let options = Options::ENABLE_TABLES | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    let mut skipping = 0usize;
    let mut paragraph: Option<String> = None;
    for event in Parser::new_ext(body, options) {
        match event {
            Event::Start(tag) if is_skipped(&tag) => skipping += 1,
            Event::End(end) if is_skipped_end(&end) => skipping -= 1,
            _ if skipping > 0 => {}
            Event::Start(Tag::Paragraph) => paragraph = Some(String::new()),
            Event::End(TagEnd::Paragraph) => {
                let found = paragraph
                    .take()
                    .map(|text| collapse(&text))
                    .unwrap_or_default();
                if !found.is_empty() {
                    return found.chars().take(EXCERPT_CHARS).collect();
                }
            }
            Event::Text(part) | Event::Code(part) => {
                if let Some(text) = paragraph.as_mut() {
                    text.push_str(&part);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(text) = paragraph.as_mut() {
                    text.push(' ');
                }
            }
            _ => {}
        }
    }
    String::new()
}

fn is_skipped(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::MetadataBlock(_) | Tag::CodeBlock(_) | Tag::Table(_) | Tag::HtmlBlock
    )
}

fn is_skipped_end(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::MetadataBlock(_) | TagEnd::CodeBlock | TagEnd::Table | TagEnd::HtmlBlock
    )
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_paragraph_with_whitespace_collapsed() {
        let text = "# Title\n\nFirst   line\nsecond\tline.\n\nNext paragraph.\n";
        assert_eq!(excerpt(text), "First line second line.");
    }

    #[test]
    fn front_matter_is_not_text_with_or_without_a_bom() {
        let text = "---\nname: x\ndescription: not this\n---\nBody text.\n";
        assert_eq!(excerpt(text), "Body text.");
        assert_eq!(excerpt(&format!("\u{feff}{text}")), "Body text.");
    }

    #[test]
    fn code_blocks_tables_and_html_are_skipped() {
        let text = "```\nfenced code\n```\n\n    indented code\n\n| a | b |\n|---|---|\n| cell | cell |\n\n\
                    <div>\nhtml block\n</div>\n\nAfter <b>them</b> all.\n";
        assert_eq!(excerpt(text), "After them all.");
    }

    #[test]
    fn inline_code_is_text_of_its_paragraph() {
        assert_eq!(
            excerpt("Run `cargo test` first.\n"),
            "Run cargo test first."
        );
    }

    #[test]
    fn the_excerpt_is_capped_in_characters() {
        let long = "가".repeat(EXCERPT_CHARS + 50);
        let got = excerpt(&long);
        assert_eq!(got.chars().count(), EXCERPT_CHARS);
        assert!(long.starts_with(&got));
    }

    #[test]
    fn no_paragraph_text_is_an_empty_excerpt() {
        assert_eq!(excerpt(""), "");
        assert_eq!(excerpt("# Only a heading\n\n```\ncode\n```\n"), "");
        assert_eq!(excerpt("<!-- a comment -->\n"), "");
    }
}
