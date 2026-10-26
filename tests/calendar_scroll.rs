//! Preserve the actual scroll widget state while opening time block panels.
use iced::advanced::widget::{Id, Operation, operation::scrollable};
use iced::{Font, Pixels, Rectangle, Size, Vector};
use iced_test::core::renderer::Headless;
use iced_test::runtime::{UserInterface, user_interface};
use timetracker::{
    app::{self, Message, State},
    calendar,
    config::Config,
    theme::Catalog,
    timeblocking::Action,
};

#[derive(Default)]
struct DayOffset(Option<f32>);

impl Operation for DayOffset {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn scrollable::Scrollable,
    ) {
        if id == Some(&Id::new("cal-day")) {
            self.0 = Some(translation.y);
        }
    }
}

#[test]
fn time_block_edit_and_view_preserve_calendar_scroll() {
    let mut renderer = iced_test::futures::futures::executor::block_on(
        <iced::Renderer as Headless>::new(Font::DEFAULT, Pixels(16.0), Some("tiny-skia")),
    )
    .expect("headless renderer");

    for width in [200.0, 800.0] {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        state.config.calendar.time_block_calendar =
            Some(state.calendar.event("standup").unwrap().calendar_id.clone());
        let mut ui = UserInterface::build(
            timetracker::ui::view(&state),
            Size::new(width, 600.0),
            user_interface::Cache::default(),
            &mut renderer,
        );
        ui.operate(
            &renderer,
            &mut scrollable::scroll_to::<()>(
                Id::new("cal-day"),
                scrollable::AbsoluteOffset {
                    x: Some(0.0),
                    y: Some(600.0),
                },
            ),
        );
        let mut offset = DayOffset::default();
        ui.operate(&renderer, &mut offset);
        let expected = offset.0.expect("calendar scroll widget");
        assert!(expected.abs() > 100.0, "test must start scrolled");
        let mut cache = ui.into_cache();

        for (message, panel_open) in [
            (Message::EventOpened("standup".into()), true),
            (Message::EventEditRequested("standup".into()), true),
            (Message::EventEditCancelled, true),
            (Message::TimeBlocking(Action::Close), false),
            (Message::TimeBlocking(Action::View("standup".into())), false),
            (Message::TimeBlocking(Action::CloseView), false),
            (Message::EventOpened("standup".into()), true),
            (Message::TimeBlocking(Action::View("standup".into())), false),
            (Message::TimeBlocking(Action::CloseView), false),
            (
                Message::TimeBlocking(Action::EditEstimate("standup".into())),
                false,
            ),
            (Message::TimeBlocking(Action::CloseEstimate), false),
        ] {
            let step = format!("{message:?}");
            let _ = app::update(&mut state, message);
            let size = Size::new(width + if panel_open { 400.0 } else { 0.0 }, 600.0);
            let mut ui =
                UserInterface::build(timetracker::ui::view(&state), size, cache, &mut renderer);
            let mut offset = DayOffset::default();
            ui.operate(&renderer, &mut offset);
            assert_eq!(
                offset.0,
                Some(expected),
                "scroll changed after {step} at {width}px"
            );
            cache = ui.into_cache();
        }
    }
}
