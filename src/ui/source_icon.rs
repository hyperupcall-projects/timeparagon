//! The small logo that marks where a task came from, drawn rather than
//! shipped as an image so it stays sharp at any size and any theme.

use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::{button, canvas as canvas_widget, tooltip};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse};

use crate::app::Message;
use crate::integrations::{Kind, Source};

/// The logo's drawing grid. Every shape below is in these units.
const WIDTH: f32 = 32.0;
const HEIGHT: f32 = 24.0;

/// The logo on its own, `height` pixels tall.
pub fn logo<'a>(kind: Kind, height: f32) -> Element<'a, Message> {
    canvas_widget(Logo { kind })
        .width(Length::Fixed(height * WIDTH / HEIGHT))
        .height(Length::Fixed(height))
        .into()
}

/// The logo as a button that opens the task's source in the browser.
pub fn link<'a>(source: &Source, height: f32) -> Element<'a, Message> {
    let open = button(logo(source.kind, height))
        .padding(0)
        .style(button::text)
        .on_press(Message::OpenEventLink(source.url.clone()));
    tooltip(
        open,
        iced::widget::text(format!("Open in {}", source.kind.name())).size(12),
        tooltip::Position::Bottom,
    )
    .gap(4)
    .style(iced::widget::container::rounded_box)
    .into()
}

struct Logo {
    kind: Kind,
}

impl canvas::Program<Message> for Logo {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let scale = bounds.height / HEIGHT;
        match self.kind {
            Kind::Gmail => gmail(&mut frame, scale),
        }
        vec![frame.into_geometry()]
    }
}

fn polygon(frame: &mut Frame, scale: f32, color: Color, points: &[(f32, f32)]) {
    let path = Path::new(|builder| {
        let mut points = points
            .iter()
            .map(|&(x, y)| Point::new(x * scale, y * scale));
        if let Some(first) = points.next() {
            builder.move_to(first);
        }
        for point in points {
            builder.line_to(point);
        }
        builder.close();
    });
    frame.fill(&path, color);
}

/// An envelope: a white card with a blue and a green edge and a red M.
fn gmail(frame: &mut Frame, scale: f32) {
    let blue = Color::from_rgb8(0x42, 0x85, 0xF4);
    let green = Color::from_rgb8(0x34, 0xA8, 0x53);
    let red = Color::from_rgb8(0xEA, 0x43, 0x35);
    let yellow = Color::from_rgb8(0xFB, 0xBC, 0x05);

    frame.fill(
        &Path::rounded_rectangle(
            Point::ORIGIN,
            iced::Size::new(WIDTH * scale, HEIGHT * scale),
            (2.5 * scale).into(),
        ),
        Color::WHITE,
    );
    polygon(
        frame,
        scale,
        blue,
        &[
            (0.0, 4.0),
            (5.5, 8.2),
            (5.5, 24.0),
            (2.0, 24.0),
            (0.0, 22.0),
        ],
    );
    polygon(
        frame,
        scale,
        green,
        &[
            (32.0, 9.0),
            (26.5, 12.8),
            (26.5, 24.0),
            (30.0, 24.0),
            (32.0, 22.0),
        ],
    );
    polygon(
        frame,
        scale,
        yellow,
        &[(32.0, 4.0), (32.0, 9.0), (26.5, 12.8), (26.5, 8.2)],
    );
    polygon(
        frame,
        scale,
        red,
        &[
            (0.0, 4.0),
            (0.0, 2.0),
            (2.0, 0.0),
            (3.5, 0.0),
            (16.0, 9.5),
            (28.5, 0.0),
            (30.0, 0.0),
            (32.0, 2.0),
            (32.0, 4.0),
            (26.5, 8.2),
            (16.0, 16.2),
            (5.5, 8.2),
        ],
    );
}
