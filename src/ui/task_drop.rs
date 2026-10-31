//! Deliver an internal task drop across the panel's two Wayland surfaces.
//!
//! Wayland keeps pointer events on the surface where the button was pressed.
//! Run the destination's normal release handling through a widget operation,
//! so its scroll offsets, clipping, day mapping and block hit testing all apply.
use crate::app::{Message, State};
use iced::advanced::{Layout, Shell, Widget, clipboard, layout, overlay, renderer, widget};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

const HOST: &str = "calendar-task-drop";

/// Convert coordinates from the sidebar's surface to the adjacent range view.
/// Both layer surfaces share the same output and vertical anchors.
pub(crate) fn range_position(
    state: &State,
    window: iced::window::Id,
    point: Point,
) -> Option<Point> {
    if !state.range_view_active() {
        return None;
    }
    let editor = state.editor_window?;
    let point = if window == editor {
        point
    } else if state.main_window == Some(window) {
        let sidebar = state.sidebar.as_ref()?;
        Point::new(
            match sidebar.side {
                crate::panel::Side::Left => point.x - state.main_size.width,
                crate::panel::Side::Right => point.x + state.editor_window_size.width,
            },
            point.y - super::editor_window_top(state),
        )
    } else {
        return None;
    };
    Rectangle::with_size(state.editor_window_size)
        .contains(point)
        .then_some(point)
}

pub(super) fn host<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(DropHost(content.into()))
}

pub(crate) fn release(task: String, position: Point) -> iced::Task<Message> {
    iced_runtime::task::widget(DropOperation {
        task,
        position,
        result: None,
    })
}

#[derive(Default)]
struct Request {
    position: Option<Point>,
    messages: Option<Vec<Message>>,
}

struct DropOperation {
    task: String,
    position: Point,
    result: Option<Message>,
}

impl widget::Operation<Message> for DropOperation {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation<Message>)) {
        operate(self);
    }

    fn custom(&mut self, id: Option<&widget::Id>, _: Rectangle, state: &mut dyn std::any::Any) {
        if id != Some(&widget::Id::new(HOST)) {
            return;
        }
        let Some(request) = state.downcast_mut::<Request>() else {
            return;
        };
        if let Some(messages) = request.messages.take() {
            self.result = messages.into_iter().find(is_drop);
        } else {
            request.position = Some(self.position);
        }
    }

    fn finish(&self) -> widget::operation::Outcome<Message> {
        widget::operation::Outcome::Some(Message::TaskDragDropped(
            self.task.clone(),
            self.result.clone().map(Box::new),
        ))
    }
}

fn is_drop(message: &Message) -> bool {
    match message {
        Message::WeekDayAction(_, message) => is_drop(message),
        Message::TimeBlocking(action) => matches!(
            action,
            crate::timeblocking::Action::DropDay
                | crate::timeblocking::Action::DropWeekList { .. }
                | crate::timeblocking::Action::DropTime(_)
                | crate::timeblocking::Action::DropBlock(_)
                | crate::timeblocking::Action::DropTask(_)
        ),
        _ => false,
    }
}

struct DropHost<'a>(Element<'a, Message>);

impl Widget<Message, Theme, Renderer> for DropHost<'_> {
    fn children(&self) -> Vec<widget::Tree> {
        vec![widget::Tree::new(&self.0)]
    }

    fn diff(&self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_ref(&self.0));
    }

    fn size(&self) -> Size<Length> {
        self.0.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.0
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let id = widget::Id::new(HOST);
        let mut request = Request::default();
        operation.custom(Some(&id), layout.bounds(), &mut request);
        if let Some(position) = request.position {
            let mut messages = Vec::new();
            self.0.as_widget_mut().update(
                &mut tree.children[0],
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                layout,
                mouse::Cursor::Available(position),
                renderer,
                &mut clipboard::Null,
                &mut Shell::new(&mut messages),
                &layout.bounds(),
            );
            request.messages = Some(messages);
            operation.custom(Some(&id), layout.bounds(), &mut request);
        } else {
            operation.traverse(&mut |operation| {
                self.0
                    .as_widget_mut()
                    .operate(&mut tree.children[0], layout, renderer, operation);
            });
        }
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.0.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.0.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.0
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut widget::Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.0.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{self, Screen, Sidebar},
        calendar::{self, Store},
        config::Config,
        panel::Side,
        theme::Catalog,
        timeblocking::{self, Action},
    };
    use chrono::{Datelike, Timelike};
    use iced_test::{Simulator, core::Settings};

    fn state(screen: Screen, monthly: bool) -> State {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let mut snapshot = calendar::sample_snapshot();
        let mut task = snapshot.events.remove(2);
        task.id = "task:drag".into();
        task.calendar_id = "tasks:inbox".into();
        task.title = "Drag this task".into();
        task.all_day = true;
        task.start = calendar::at_clock(timeblocking::UNDATED, 0, 0);
        task.end = task.start + chrono::Duration::days(1);
        snapshot.events = vec![task];
        state.config.calendar.inbox_list = Some("tasks:inbox".into());
        state.config.calendar.backlog_list = Some("tasks:inbox".into());
        state.week_anchor = snapshot.day;
        state.calendar = Store::with_snapshot(snapshot);
        state.screen = screen;
        state.weekly = !monthly;
        state.monthly = monthly;
        state.main_window = Some(iced::window::Id::unique());
        state.editor_window = Some(iced::window::Id::unique());
        state.main_size = Size::new(300.0, 900.0);
        state.editor_window_size = Size::new(1200.0, 880.0);
        state.sidebar = Some(Sidebar {
            width: 300.0,
            side: Side::Left,
        });
        state
    }

    /// Exercise the same operation used by the multi-window runtime, including
    /// the host's normal scrollable and canvas event dispatch.
    fn drop_at(state: &mut State, position: Point) -> Message {
        drop_scrolled(state, position, 0.0, 0.0)
    }

    fn drop_scrolled(state: &mut State, position: Point, x: f32, y: f32) -> Message {
        use iced::futures::{StreamExt, executor::block_on};
        use iced_test::core::renderer::Headless;
        let main = state.main_window.unwrap();
        let source = Point::new(
            position.x + state.main_size.width,
            position.y + state.screen_margin(),
        );
        let _ = app::update(state, Message::TaskDragMoved(main, source));
        let task = app::update(state, Message::TaskDragReleased(main));
        let mut renderer = block_on(Renderer::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            Some("tiny-skia"),
        ))
        .unwrap();
        let mut ui = iced_runtime::UserInterface::build(
            super::super::editor_window(state),
            state.editor_window_size,
            iced_runtime::user_interface::Cache::default(),
            &mut renderer,
        );
        if x != 0.0 {
            ui.operate(
                &renderer,
                &mut widget::operation::scrollable::scroll_to(
                    widget::Id::new("cal-week"),
                    widget::operation::scrollable::AbsoluteOffset {
                        x: Some(x),
                        y: None,
                    },
                ),
            );
        }
        if y != 0.0 {
            ui.operate(
                &renderer,
                &mut widget::operation::scrollable::scroll_to(
                    widget::Id::new(if state.config.calendar.weekly_task_list {
                        "cal-week"
                    } else {
                        "cal-week-day"
                    }),
                    widget::operation::scrollable::AbsoluteOffset {
                        x: None,
                        y: Some(y),
                    },
                ),
            );
        }
        block_on(async {
            let mut stream = iced_runtime::task::into_stream(task).unwrap();
            while let Some(action) = stream.next().await {
                match action {
                    iced_runtime::Action::Widget(mut operation) => {
                        ui.operate(&renderer, operation.as_mut());
                        let _ = operation.finish();
                    }
                    iced_runtime::Action::Output(message @ Message::TaskDragDropped(..)) => {
                        return message;
                    }
                    _ => {}
                }
            }
            panic!("drop operation did not complete")
        })
    }

    fn start_drag(state: &mut State) {
        let mut ui = Simulator::with_size(
            Settings::default(),
            state.main_size,
            super::super::view(state),
        );
        let bounds = ui.find("Drag this task").unwrap().bounds();
        let origin = bounds.center();
        ui.point_at(origin);
        ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))]);
        let position = origin + Vector::new(10.0, 0.0);
        ui.point_at(position);
        ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position })]);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, Message::EventOpened(_)))
        );
        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Message::TimeBlocking(Action::Drag(_, _))))
        );
        for message in messages {
            let _ = app::update(state, message);
        }
        assert_eq!(state.timeblocking.dragging.as_deref(), Some("task:drag"));
    }

    #[test]
    fn inbox_and_backlog_drag_into_month_dates_and_empty_week_list_columns() {
        for screen in [Screen::Inbox, Screen::Backlog] {
            for monthly in [false, true] {
                let mut state = state(screen, monthly);
                state.config.calendar.weekly_task_list = true;
                start_drag(&mut state);
                let anchor = state.week_anchor;
                let day = if monthly {
                    calendar::month_grid(anchor)[10]
                } else {
                    state.calendar_dates()[3]
                };
                let mut ui = Simulator::with_size(
                    Settings::default(),
                    state.editor_window_size,
                    super::super::editor_window(&state),
                );
                let label = if monthly {
                    day.day().to_string()
                } else {
                    day.format("%B %-d").to_string()
                };
                let bounds = ui.find(label.as_str()).unwrap().bounds();
                let point = bounds.center() + Vector::new(0.0, 45.0);
                drop(ui);
                let main = state.main_window.unwrap();
                let source = Point::new(
                    point.x + state.main_size.width,
                    point.y + state.screen_margin(),
                );
                let _ = app::update(&mut state, Message::TaskDragMoved(main, source));
                assert!(
                    range_position(&state, main, source)
                        .unwrap()
                        .distance(point)
                        < 0.001
                );
                let message = drop_at(&mut state, point);
                assert!(
                    matches!(&message, Message::TaskDragDropped(_, Some(message))
                    if matches!(&**message, Message::WeekDayAction(date, inner)
                        if *date == day && matches!(&**inner, Message::TimeBlocking(Action::DropDay | Action::DropWeekList { .. })))),
                    "{screen:?}, monthly={monthly}, point={point:?}: {message:?}"
                );
                // Block the save path: test only UI routing, never remote writes.
                state
                    .timeblocking
                    .busy
                    .insert("test-no-remote-write".into());
                let _ = app::update(&mut state, message);
                assert_eq!(state.calendar.day, day);
                assert_eq!(state.week_anchor, anchor);
                assert_eq!(state.screen, screen);
                assert_eq!(state.monthly, monthly);
                assert_eq!(state.weekly, !monthly);
                assert!(state.timeblocking.dragging.is_none());
                assert!(state.timeblocking.drag_pointer.is_none());
                assert_eq!(state.timeblocking.busy.len(), 1);
            }
        }
    }

    #[test]
    fn week_grid_drop_uses_destination_day_and_snapped_time() {
        let mut state = state(Screen::Inbox, false);
        start_drag(&mut state);
        let day = state.calendar_dates()[3];
        let mut ui = Simulator::with_size(
            Settings::default(),
            state.editor_window_size,
            super::super::editor_window(&state),
        );
        let bounds = ui
            .find(day.format("%a %b %-d").to_string().as_str())
            .unwrap()
            .bounds();
        let point = Point::new(bounds.center_x(), 300.0);
        drop(ui);
        let message = drop_at(&mut state, point);
        assert!(
            matches!(&message, Message::TaskDragDropped(_, Some(message))
            if matches!(&**message, Message::WeekDayAction(date, message)
                if *date == day && matches!(&**message, Message::TimeBlocking(Action::DropTime(time))
                    if time.date_naive() == day && time.minute() % 15 == 0)))
        );
    }

    #[test]
    fn week_grid_drop_honors_horizontal_and_vertical_scroll() {
        let mut state = state(Screen::Backlog, false);
        state.config.calendar.weekly_day_width = 200;
        state.config.views.calendar_week_width = Some(800.0);
        start_drag(&mut state);
        let day = state.calendar_dates()[2];
        // A column and an hour scrolled off-screen must affect hit testing.
        let message = drop_scrolled(&mut state, Point::new(250.0, 300.0), 200.0, 60.0);
        assert!(
            matches!(&message, Message::TaskDragDropped(_, Some(message))
            if matches!(&**message, Message::WeekDayAction(date, message)
                if *date == day && matches!(&**message, Message::TimeBlocking(Action::DropTime(time))
                    if time.date_naive() == day && time.minute() % 15 == 0))),
            "{message:?}"
        );
    }

    #[test]
    fn week_board_drop_honors_scroll_offsets_and_precise_card_placement() {
        let mut state = state(Screen::Backlog, false);
        state.config.calendar.weekly_task_list = true;
        state.config.views.calendar_week_width = Some(1200.0);
        start_drag(&mut state);
        let days = state.calendar_dates();
        let destination = days[3];
        let mut snapshot = state.calendar.cached_snapshot().unwrap();
        let template = snapshot.events[0].clone();
        for index in 0..20 {
            let mut task = template.clone();
            task.id = format!("task:destination-{index}");
            task.title = format!("Destination row {index}");
            task.all_day = false;
            task.start =
                calendar::at_clock(destination, 6, 0) + chrono::Duration::minutes(30 * index);
            task.end = task.start + chrono::Duration::minutes(30);
            snapshot.events.push(task);
        }
        state.calendar.apply_week_snapshot(snapshot, &days);
        let mut ui = Simulator::with_size(
            Settings::default(),
            state.editor_window_size,
            super::super::editor_window(&state),
        );
        let title = ui.find("Destination row 4").unwrap().bounds();
        let point = title.center() - Vector::new(240.0, 250.0);
        drop(ui);
        let message = drop_scrolled(&mut state, point, 240.0, 250.0);
        assert!(
            matches!(&message, Message::TaskDragDropped(_, Some(message))
            if matches!(&**message, Message::WeekDayAction(day, inner)
                if *day == destination && matches!(&**inner, Message::TimeBlocking(Action::DropWeekList { target: Some(id), .. })
                    if id == "task:destination-4"))),
            "{point:?}: {message:?}"
        );
    }

    #[test]
    fn bridge_cancels_outside_calendar_and_ignores_results_after_escape() {
        let mut state = state(Screen::Inbox, true);
        start_drag(&mut state);
        let main = state.main_window.unwrap();
        assert!(range_position(&state, main, Point::new(20.0, 200.0)).is_none());
        let no_target = drop_at(&mut state, Point::new(5.0, 5.0));
        assert!(matches!(&no_target, Message::TaskDragDropped(_, None)));
        let _ = app::update(&mut state, no_target);
        assert!(state.timeblocking.dragging.is_none());
        assert!(state.timeblocking.busy.is_empty());
        let _ = app::update(
            &mut state,
            Message::TaskDragDropped(
                "task:drag".into(),
                Some(Box::new(Message::TimeBlocking(Action::DropDay))),
            ),
        );
        assert!(state.timeblocking.busy.is_empty());
    }

    #[test]
    fn coordinates_handle_both_panel_sides_and_exclude_other_windows() {
        let mut state = state(Screen::Inbox, true);
        let main = state.main_window.unwrap();
        let editor = state.editor_window.unwrap();
        let point = Point::new(150.0, 250.0);
        let top = state.screen_margin();
        assert_eq!(
            range_position(&state, main, Point::new(450.0, 250.0 + top)),
            Some(point)
        );
        state.sidebar.as_mut().unwrap().side = Side::Right;
        assert_eq!(
            range_position(&state, main, Point::new(-1050.0, 250.0 + top)),
            Some(point)
        );
        assert_eq!(range_position(&state, editor, point), Some(point));
        assert!(range_position(&state, iced::window::Id::unique(), point).is_none());
        state.monthly = false;
        assert!(range_position(&state, editor, point).is_none());
    }
}
