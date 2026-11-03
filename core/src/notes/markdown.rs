//! The small part of Markdown a note can show.
//!
//! Headings, bullet, numbered and check lists, quotes, code, rules, bold,
//! italics, strikethrough and links. Anything else (tables, footnotes, HTML,
//! images) is shown as the plain text it is written in, so nothing the user
//! typed disappears.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A run of text sharing one style.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// Only web and mail links: a note must not be able to launch anything else.
    pub link: Option<String>,
}

/// What starts a list item's line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Bullet,
    Number(u64),
    Check(bool),
    /// A further paragraph of the item above.
    Continued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        text: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Item {
        depth: usize,
        marker: Marker,
        text: Vec<Inline>,
    },
    Quote(Vec<Inline>),
    Code(String),
    Rule,
}

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    strike: u32,
    link: Option<String>,
}

impl Style {
    fn inline(&self, text: &str, code: bool) -> Inline {
        Inline {
            text: text.to_owned(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code,
            link: self.link.clone(),
        }
    }
}

/// One entry per list that is open: its next number, if it is numbered.
struct List {
    next: Option<u64>,
    /// The item in progress has already drawn its marker.
    marked: bool,
    /// The item's own marker, until it is drawn.
    marker: Marker,
}

fn safe_link(url: &str) -> Option<String> {
    let scheme = url.split_once(':')?.0.to_ascii_lowercase();
    matches!(scheme.as_str(), "http" | "https" | "mailto").then(|| url.to_owned())
}

/// Parses a note's text into what the View tab draws.
pub fn parse(source: &str) -> Vec<Block> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut blocks = Vec::new();
    let mut inline: Vec<Inline> = Vec::new();
    let mut style = Style::default();
    let mut lists: Vec<List> = Vec::new();
    let mut quotes = 0_u32;
    let mut heading: Option<u8> = None;
    let mut code: Option<String> = None;

    fn flush(
        blocks: &mut Vec<Block>,
        inline: &mut Vec<Inline>,
        lists: &mut [List],
        quotes: u32,
        heading: Option<u8>,
    ) {
        if inline.is_empty() {
            return;
        }
        let text = std::mem::take(inline);
        let block = if let Some(level) = heading {
            Block::Heading { level, text }
        } else if let Some(list) = lists.last_mut() {
            let marker = if list.marked {
                Marker::Continued
            } else {
                list.marked = true;
                list.marker
            };
            Block::Item {
                depth: lists.len() - 1,
                marker,
                text,
            }
        } else if quotes > 0 {
            Block::Quote(text)
        } else {
            Block::Paragraph(text)
        };
        blocks.push(block);
    }

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
                heading = None;
            }
            Event::End(TagEnd::Paragraph) => {
                flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
            }
            Event::Start(Tag::BlockQuote(_)) => quotes += 1,
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
                quotes = quotes.saturating_sub(1);
            }
            Event::Start(Tag::List(first)) => {
                // A tight parent item has no paragraph to end its own text.
                flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
                lists.push(List {
                    next: first,
                    marked: false,
                    marker: Marker::Bullet,
                });
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                if let Some(list) = lists.last_mut() {
                    list.marked = false;
                    list.marker = match list.next {
                        Some(number) => {
                            list.next = Some(number + 1);
                            Marker::Number(number)
                        }
                        None => Marker::Bullet,
                    };
                }
            }
            Event::End(TagEnd::Item) => {
                flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
            }
            Event::TaskListMarker(done) => {
                if let Some(list) = lists.last_mut() {
                    list.marker = Marker::Check(done);
                }
            }
            Event::Start(Tag::Strong) => style.bold += 1,
            Event::End(TagEnd::Strong) => style.bold = style.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => style.italic += 1,
            Event::End(TagEnd::Emphasis) => style.italic = style.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => style.strike += 1,
            Event::End(TagEnd::Strikethrough) => style.strike = style.strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => style.link = safe_link(&dest_url),
            Event::End(TagEnd::Link) => style.link = None,
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(text) = code.take() {
                    blocks.push(Block::Code(text.trim_end_matches('\n').to_owned()));
                }
            }
            Event::Text(text) => match &mut code {
                Some(block) => block.push_str(&text),
                None => inline.push(style.inline(&text, false)),
            },
            Event::Code(text) => inline.push(style.inline(&text, true)),
            Event::SoftBreak => inline.push(style.inline(" ", false)),
            Event::HardBreak => inline.push(style.inline("\n", false)),
            // Shown as written rather than interpreted.
            Event::Html(text) | Event::InlineHtml(text) => {
                inline.push(style.inline(text.trim_end_matches('\n'), false));
                if lists.is_empty() && heading.is_none() {
                    flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
                }
            }
            Event::Rule => blocks.push(Block::Rule),
            _ => {}
        }
    }
    flush(&mut blocks, &mut inline, &mut lists, quotes, heading);
    blocks
}

/// The note's words without its markup, for a card's preview.
pub fn plain(source: &str, limit: usize) -> String {
    let words = |text: &[Inline]| text.iter().map(|run| run.text.as_str()).collect::<String>();
    let mut lines = Vec::new();
    for block in parse(source) {
        lines.push(match block {
            Block::Heading { text, .. } | Block::Paragraph(text) | Block::Quote(text) => {
                words(&text)
            }
            Block::Item { marker, text, .. } => match marker {
                Marker::Continued => words(&text),
                _ => format!("• {}", words(&text)),
            },
            Block::Code(text) => text,
            Block::Rule => continue,
        });
    }
    let text = lines.join("\n");
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &[Inline]) -> String {
        text.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn headings_paragraphs_and_inline_styles() {
        let blocks = parse("# Title\n\nSome **bold**, *italic*, ~~gone~~ and `code`.\n\n---\n");
        assert!(matches!(&blocks[0], Block::Heading { level: 1, text } if words(text) == "Title"));
        let Block::Paragraph(runs) = &blocks[1] else {
            panic!("{blocks:?}");
        };
        assert!(runs.iter().any(|run| run.bold && run.text == "bold"));
        assert!(runs.iter().any(|run| run.italic && run.text == "italic"));
        assert!(runs.iter().any(|run| run.strike && run.text == "gone"));
        assert!(runs.iter().any(|run| run.code && run.text == "code"));
        assert_eq!(blocks[2], Block::Rule);
    }

    #[test]
    fn bold_italic_nests() {
        let blocks = parse("***both***");
        let Block::Paragraph(runs) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        assert!(runs[0].bold && runs[0].italic);
    }

    #[test]
    fn lists_keep_their_depth_numbers_and_checks() {
        let blocks =
            parse("- one\n  - inner\n- two\n\n3. third\n4. fourth\n\n- [x] done\n- [ ] open\n");
        let items: Vec<(usize, Marker, String)> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Item {
                    depth,
                    marker,
                    text,
                } => Some((*depth, *marker, words(text))),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            [
                (0, Marker::Bullet, "one".to_owned()),
                (1, Marker::Bullet, "inner".to_owned()),
                (0, Marker::Bullet, "two".to_owned()),
                (0, Marker::Number(3), "third".to_owned()),
                (0, Marker::Number(4), "fourth".to_owned()),
                (0, Marker::Check(true), "done".to_owned()),
                (0, Marker::Check(false), "open".to_owned()),
            ]
        );
    }

    #[test]
    fn a_second_paragraph_in_an_item_does_not_repeat_its_marker() {
        let blocks = parse("1. first\n\n   more of it\n2. second\n");
        assert!(matches!(
            &blocks[1],
            Block::Item { marker: Marker::Continued, text, .. } if words(text) == "more of it"
        ));
        assert!(matches!(
            &blocks[2],
            Block::Item {
                marker: Marker::Number(2),
                ..
            }
        ));
    }

    #[test]
    fn quotes_and_code_blocks() {
        let blocks = parse("> quoted\n\n```\nlet a = 1;\n```\n");
        assert!(matches!(&blocks[0], Block::Quote(text) if words(text) == "quoted"));
        assert_eq!(blocks[1], Block::Code("let a = 1;".to_owned()));
    }

    #[test]
    fn only_web_and_mail_links_are_followed() {
        let blocks = parse("[web](https://a.example) [run](file:///bin/sh) [mail](mailto:a@b.c)");
        let Block::Paragraph(runs) = &blocks[0] else {
            panic!("{blocks:?}");
        };
        let link = |label: &str| {
            runs.iter()
                .find(|run| run.text == label)
                .unwrap()
                .link
                .clone()
        };
        assert_eq!(link("web").as_deref(), Some("https://a.example"));
        assert_eq!(link("run"), None);
        assert_eq!(link("mail").as_deref(), Some("mailto:a@b.c"));
    }

    #[test]
    fn unsupported_syntax_stays_readable() {
        let blocks =
            parse("| a | b |\n|---|---|\n\n<b>raw</b>\n\n![alt](https://a.example/i.png)\n");
        let all: String = blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph(text) => words(text),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("| a | b |"), "{all}");
        assert!(all.contains("<b>"), "{all}");
        assert!(all.contains("raw"), "{all}");
        assert!(all.contains("alt"), "{all}");
    }

    #[test]
    fn the_preview_drops_markup_and_stops_at_the_limit() {
        assert_eq!(plain("# Hi\n\n- **a**\n- b\n", 100), "Hi\n• a\n• b");
        assert_eq!(plain("abcdef", 3), "abc…");
        assert_eq!(plain("", 3), "");
    }
}
