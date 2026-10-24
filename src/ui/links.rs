//! Shared link detection for displayed text and links alongside editable fields.

use iced::widget::{column, rich_text, span};
use iced::{Element, Fill};
use linkify::{LinkFinder, LinkKind};

use crate::app::Message;

fn finder() -> LinkFinder {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]).url_must_have_scheme(false);
    finder
}

fn target(value: &str) -> Option<String> {
    if let Some((scheme, _)) = value.split_once(":") {
        if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            Some(value.to_owned())
        } else {
            None
        }
    } else {
        Some(format!("https://{value}"))
    }
}

/// Keep prose and wrapping intact; only detected web links are interactive.
pub fn text<'a>(value: impl Into<String>, size: f32) -> Element<'a, Message> {
    let value = value.into();
    if !finder()
        .links(&value)
        .any(|link| target(link.as_str()).is_some())
    {
        return iced::widget::text(value).size(size).width(Fill).into();
    }
    let spans: Vec<_> = finder()
        .spans(&value)
        .map(|part| {
            let link = part.kind().and_then(|_| target(part.as_str()));
            span(part.as_str().to_owned())
                .underline(link.is_some())
                .link_maybe(link)
        })
        .collect();

    rich_text(spans)
        .size(size)
        .width(Fill)
        .on_link_click(Message::OpenEventLink)
        .into()
}

/// Editors retain normal cursor behavior, with their links available below.
pub fn for_editor<'a>(value: &str, size: f32) -> Option<Element<'a, Message>> {
    let mut seen = std::collections::HashSet::new();
    let links: Vec<_> = finder()
        .links(value)
        .filter_map(|link| {
            let url = target(link.as_str())?;
            seen.insert(url).then(|| text(link.as_str(), size))
        })
        .collect();
    if links.is_empty() {
        None
    } else {
        Some(column(links).spacing(2.0).width(Fill).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_a_link_emits_the_browser_message() {
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(200.0, 80.0),
            text("www.example.org", 16.0),
        );
        ui.point_at(iced::Point::new(10.0, 10.0));
        ui.simulate(iced_test::simulator::click());
        assert!(ui.into_messages().any(|message| matches!(
            message,
            Message::OpenEventLink(url) if url == "https://www.example.org"
        )));
    }

    #[test]
    fn detects_links_without_surrounding_punctuation() {
        let value = "See (https://example.com/a?q=1&b=2), www.example.org.\nhttps://en.wikipedia.org/wiki/Link_(film)";
        let links: Vec<_> = finder().links(value).map(|link| link.as_str()).collect();
        assert_eq!(
            links,
            [
                "https://example.com/a?q=1&b=2",
                "www.example.org",
                "https://en.wikipedia.org/wiki/Link_(film)"
            ]
        );
    }

    #[test]
    fn normalizes_web_addresses_and_rejects_other_schemes() {
        assert_eq!(
            target("www.example.org"),
            Some("https://www.example.org".into())
        );
        assert_eq!(
            target("https://example.org"),
            Some("https://example.org".into())
        );
        assert_eq!(target("javascript:alert(1)"), None);
        assert_eq!(target("file:///tmp/test"), None);
    }
}
