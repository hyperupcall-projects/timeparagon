//! Ctrl+wheel for the month and two-week grids.
//!
//! Those grids are made of widgets rather than one canvas, and their cells
//! scroll on their own. An invisible layer on top sees the wheel before any of
//! them, takes it while Ctrl is held and otherwise lets everything through.

use super::*;

/// `grid` with the wheel zooming it while Ctrl is held. `scroll_top` is how far
/// the grid is scrolled, which turns the pointer into a row of the grid.
pub(super) fn zoom_wheel_layer<'a>(
    grid: Element<'a, Message>,
    scroll_top: f32,
) -> Element<'a, Message> {
    stack![
        grid,
        canvas_widget(ZoomWheel { scroll_top })
            .width(Fill)
            .height(Fill)
    ]
    .into()
}

struct ZoomWheel {
    scroll_top: f32,
}

impl canvas::Program<Message> for ZoomWheel {
    /// Whether Ctrl is held.
    type State = bool;

    fn update(
        &self,
        held: &mut bool,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                *held = modifiers.command();
                None
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) if *held => {
                let point = cursor.position_in(bounds)?;
                let steps = zoom_steps(*delta);
                if steps == 0.0 {
                    return None;
                }
                Some(
                    canvas::Action::publish(Message::Calendar(CalendarMessage::CalendarZoomed {
                        factor: WHEEL_ZOOM.powf(steps),
                        anchor: Some(self.scroll_top + point.y),
                    }))
                    .and_capture(),
                )
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _held: &bool,
        _renderer: &Renderer,
        _theme: &Theme,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        Vec::new()
    }
}
