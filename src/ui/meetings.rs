//! Meeting actions for structured conference links and links in invitation text.

use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::{button, canvas as canvas_widget, row, text};
use iced::{Center, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, mouse};
use linkify::{LinkFinder, LinkKind};
use url::Url;

use super::layout::Metrics;
use crate::{app::Message, calendar::Event};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Zoom,
    Meet,
    Other,
}

impl Provider {
    fn label(self) -> &'static str {
        match self {
            Self::Zoom => "Open in Zoom",
            Self::Meet => "Open in Google Meet",
            Self::Other => "Join meeting",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Meeting {
    provider: Provider,
    url: String,
}

fn provider(url: &Url) -> Provider {
    match url.host_str().unwrap_or_default() {
        "meet.google.com" => Provider::Meet,
        host if ["zoom.us", "zoomgov.com"]
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))) =>
        {
            Provider::Zoom
        }
        _ => Provider::Other,
    }
}

/// Keep full invite URLs, including passwords and authentication parameters.
/// Structured conference links may be another provider (or a dial-in number);
/// only Zoom and Meet links are promoted from free text.
pub(super) fn links(event: &Event) -> Vec<Meeting> {
    let mut meetings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |value: &str, structured: bool| {
        let value = value.trim();
        let parsed = Url::parse(value).or_else(|_| Url::parse(&format!("https://{value}")));
        let Ok(url) = parsed else { return };
        let provider = provider(&url);
        let web = matches!(url.scheme(), "https" | "http") && url.host_str().is_some();
        if !(web || structured && url.scheme() == "tel") {
            return;
        }
        if !url.username().is_empty() || url.password().is_some() {
            return;
        }
        if !structured && (provider == Provider::Other || url.path().trim_matches('/').is_empty()) {
            return;
        }
        let url = url.to_string();
        if seen.insert(url.clone()) {
            meetings.push(Meeting { provider, url });
        }
    };
    if !event.details.meeting.is_empty() {
        add(&event.details.meeting, true);
    }
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]).url_must_have_scheme(false);
    for value in [
        &event.title,
        &event.details.location,
        &event.details.description,
    ] {
        for link in finder.links(value) {
            add(link.as_str(), false);
        }
    }
    meetings
}

pub(super) fn join_button<'a>(meeting: Meeting, metrics: &Metrics) -> Element<'a, Message> {
    let icon = canvas_widget(Camera(meeting.provider)).width(24).height(18);
    button(
        row![icon, text(meeting.provider.label()).size(metrics.label())]
            .spacing(metrics.spacing)
            .align_y(Center),
    )
    .width(Fill)
    .on_press(Message::OpenEventLink(meeting.url))
    .style(crate::ui::controls::secondary)
    .into()
}

/// Small vector camera marks: Zoom blue and Google Meet's four colors.
struct Camera(Provider);

impl canvas::Program<Message> for Camera {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let sx = bounds.width / 24.0;
        let sy = bounds.height / 18.0;
        let rectangle =
            |x, y, w, h| Path::rectangle(Point::new(x * sx, y * sy), Size::new(w * sx, h * sy));
        let lens = Path::new(|p| {
            p.move_to(Point::new(17.0 * sx, 6.0 * sy));
            p.line_to(Point::new(24.0 * sx, 2.0 * sy));
            p.line_to(Point::new(24.0 * sx, 16.0 * sy));
            p.line_to(Point::new(17.0 * sx, 12.0 * sy));
            p.close();
        });
        if self.0 == Provider::Meet {
            frame.fill(
                &rectangle(0.0, 2.0, 5.0, 14.0),
                Color::from_rgb8(0x42, 0x85, 0xF4),
            );
            frame.fill(
                &rectangle(5.0, 2.0, 12.0, 5.0),
                Color::from_rgb8(0xFB, 0xBC, 0x05),
            );
            frame.fill(
                &rectangle(5.0, 7.0, 12.0, 9.0),
                Color::from_rgb8(0x34, 0xA8, 0x53),
            );
            frame.fill(
                &rectangle(5.0, 2.0, 5.0, 5.0),
                Color::from_rgb8(0xEA, 0x43, 0x35),
            );
            frame.fill(&lens, Color::from_rgb8(0x34, 0xA8, 0x53));
        } else {
            let color = if self.0 == Provider::Zoom {
                Color::from_rgb8(0x2D, 0x8C, 0xFF)
            } else {
                theme.extended_palette().primary.base.text
            };
            frame.fill(
                &Path::rounded_rectangle(
                    Point::new(0.0, 2.0 * sy),
                    Size::new(17.0 * sx, 14.0 * sy),
                    (3.0 * sy).into(),
                ),
                color,
            );
            frame.fill(&lens, color);
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> Event {
        crate::calendar::sample_snapshot()
            .events
            .into_iter()
            .next()
            .unwrap()
    }

    #[test]
    fn finds_invites_in_title_location_and_description_and_deduplicates() {
        let mut event = event();
        event.title = "Call: meet.google.com/abc-defg-hij".into();
        event.details = Default::default();
        event.details.meeting = "https://meet.google.com/abc-defg-hij".into();
        event.details.location = "https://us02web.zoom.us/j/123456?pwd=secret&authuser=1".into();
        event.details.description = "Join (https://us02web.zoom.us/j/123456?pwd=secret&authuser=1). Or https://agency.zoomgov.com/j/987".into();
        let links = links(&event);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].provider, Provider::Meet);
        assert_eq!(links[1].provider, Provider::Zoom);
        assert_eq!(
            links[1].url,
            "https://us02web.zoom.us/j/123456?pwd=secret&authuser=1"
        );
        assert_eq!(links[2].provider, Provider::Zoom);
    }

    #[test]
    fn ignores_unrelated_links_and_lookalike_hosts() {
        let mut event = event();
        event.details = Default::default();
        event.title = "https://zoom.us.evil.example/j/123 https://notzoom.us/j/123 https://meet.google.com.evil.example/abc https://example.com https://zoom.us".into();
        assert!(links(&event).is_empty());
    }

    #[test]
    fn preserves_other_conference_providers_and_dial_in() {
        let mut event = event();
        event.title.clear();
        event.details = Default::default();
        for url in [
            "https://teams.microsoft.com/l/meetup-join/abc",
            "tel:+15551234567",
        ] {
            event.details.meeting = url.into();
            let links = links(&event);
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].provider, Provider::Other);
            assert_eq!(links[0].url, url);
        }
        event.details.meeting = "javascript:alert(1)".into();
        assert!(links(&event).is_empty());
    }

    #[test]
    fn icon_buttons_emit_the_full_invite_url() {
        let metrics = Metrics::of(Size::new(360.0, 800.0));
        for provider in [Provider::Zoom, Provider::Meet, Provider::Other] {
            let url = "https://us02web.zoom.us/j/123?pwd=abc&authuser=1";
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(360.0, 100.0),
                join_button(
                    Meeting {
                        provider,
                        url: url.into(),
                    },
                    &metrics,
                ),
            );
            ui.click(provider.label()).unwrap();
            assert!(
                ui.into_messages()
                    .any(|m| matches!(m, Message::OpenEventLink(value) if value == url))
            );
        }
    }
}
