//! The full-screen flash overlay.
//!
//! This is a second, borderless, transparent window. Its theme paints nothing
//! of its own, so the only thing on screen is the tinted container below,
//! with everything behind it still visible through the alpha.

use iced::widget::{container, mouse_area};
use iced::{Color, Element, Fill};

use crate::app::{Message, State};

pub fn view(state: &State) -> Element<'_, Message> {
    let alpha = state
        .flash
        .as_ref()
        .and_then(|flash| flash.alpha(state.now))
        .unwrap_or(0.0);

    let color = state
        .flash
        .as_ref()
        .map(|flash| flash.settings.color.0)
        .unwrap_or(Color::TRANSPARENT);

    let passthrough = state
        .flash
        .as_ref()
        .is_some_and(|flash| flash.settings.passthrough);

    let tint = container(iced::widget::Space::new().width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(move |_theme| container::Style {
            background: Some(color.scale_alpha(alpha).into()),
            ..container::Style::default()
        });

    if passthrough {
        // Clicks are going through to whatever is underneath, so there is
        // nothing to attach a press handler to; the keyboard and the
        // envelope's own expiry are what end the flash.
        tint.into()
    } else {
        mouse_area(tint)
            .on_press(Message::Dismiss)
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }
}
