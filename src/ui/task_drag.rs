//! Whole-row task dragging that preserves button clicks until the pointer moves.
use crate::{app::Message, timeblocking::Action};
use iced::{Element, Length, Rectangle, Renderer, Size, Theme, mouse};

#[derive(Default)]
struct Gesture {
    origin: Option<iced::Point>,
    active: bool,
    /// Set once the app has rebuilt this row knowing it is the one in the air.
    /// A later rebuild without it means the drag was dropped or cancelled, so
    /// the row can give up the pointer.
    acknowledged: bool,
}

/// Wraps a task card so the whole row can be dragged.
///
/// `dragging` is the id of the task currently in the air, if any. This lets a
/// row tell whether it is the one being lifted (and so should leave its place)
/// from whether some other row is being dragged (so its buttons hold still).
pub(super) fn task_drag<'a>(
    content: impl Into<Element<'a, Message>>,
    id: &str,
    dragging: Option<&str>,
) -> Element<'a, Message> {
    Element::new(DraggableTask {
        content: content.into(),
        id: id.to_owned(),
        lifted: dragging == Some(id),
        dragging: dragging.is_some(),
    })
}

struct DraggableTask<'a> {
    content: Element<'a, Message>,
    id: String,
    /// True when this very row is the one being dragged.
    lifted: bool,
    /// True while any task is being dragged.
    dragging: bool,
}

impl iced::advanced::widget::Widget<Message, Theme, Renderer> for DraggableTask<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<Gesture>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(Gesture::default())
    }
    fn children(&self) -> Vec<iced::advanced::widget::Tree> {
        vec![iced::advanced::widget::Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        renderer: &Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                operation,
            );
        });
    }

    fn update(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let gesture = tree.state.downcast_mut::<Gesture>();
        // Once the app has rebuilt this row as the one in the air, a later
        // rebuild without it means the drag was dropped or cancelled, so give
        // up the pointer and let the next release land as a plain release.
        if self.lifted {
            gesture.acknowledged = true;
        } else if gesture.acknowledged {
            gesture.acknowledged = false;
            gesture.active = false;
            gesture.origin = None;
        }
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(layout.bounds()) && cursor.is_over(*viewport) =>
            {
                gesture.origin = cursor.position();
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if gesture
                    .origin
                    .is_some_and(|origin| position.distance(origin) >= 4.0)
                    && !gesture.active
                    && !self.dragging
                {
                    gesture.active = true;
                    // Clear pressed buttons so releasing a drag cannot also click them.
                    tree.children[0] = iced::advanced::widget::Tree::new(&self.content);
                    shell.publish(Message::TimeBlocking(Action::Drag(
                        self.id.clone(),
                        *position,
                    )));
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let dragging = self.lifted || gesture.active;
                gesture.origin = None;
                gesture.active = false;
                if dragging {
                    if cursor.is_over(layout.bounds()) && cursor.is_over(*viewport) {
                        shell.publish(Message::TimeBlocking(Action::DropTask(self.id.clone())));
                        shell.capture_event();
                    }
                    return;
                }
            }
            _ => {}
        }
        if !self.dragging && !gesture.active {
            self.content.as_widget_mut().update(
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
    }

    fn draw(
        &self,
        tree: &iced::advanced::widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        // The row that is in the air leaves its place, so the drag reads as
        // picking the task up rather than copying it. The space stays reserved
        // so the rows around it do not jump.
        if self.lifted {
            return;
        }
        self.content.as_widget().draw(
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
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.dragging || tree.state.downcast_ref::<Gesture>().active {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(layout.bounds()) && cursor.is_over(*viewport) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::None
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
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
    use iced::{
        Point,
        widget::{button, container, row, text},
    };

    #[test]
    fn the_whole_card_drags_without_activating_its_buttons() {
        for x in [5.0, 60.0, 170.0, 295.0] {
            for drag in [false, true] {
                let content = container(row![
                    button("Task title")
                        .width(180)
                        .on_press(Message::EventOpened("task".into())),
                    text("30m")
                ])
                .padding(10)
                .width(300)
                .height(50);
                let mut ui = iced_test::Simulator::with_size(
                    iced_test::core::Settings::default(),
                    Size::new(300.0, 100.0),
                    task_drag(content, "task", None),
                );
                ui.point_at(Point::new(x, 20.0));
                ui.simulate([iced::Event::Mouse(mouse::Event::ButtonPressed(
                    mouse::Button::Left,
                ))]);
                if drag {
                    let position = Point::new(x, 30.0);
                    ui.point_at(position);
                    ui.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position })]);
                }
                ui.simulate([iced::Event::Mouse(mouse::Event::ButtonReleased(
                    mouse::Button::Left,
                ))]);
                let messages: Vec<_> = ui.into_messages().collect();
                assert_eq!(
                    messages.iter().any(
                        |m| matches!(m, Message::TimeBlocking(Action::Drag(id, _)) if id == "task")
                    ),
                    drag
                );
                assert_eq!(
                    messages
                        .iter()
                        .any(|m| matches!(m, Message::EventOpened(_))),
                    !drag && (10.0..190.0).contains(&x)
                );
            }
        }
    }

    #[test]
    fn dropping_on_a_button_reorders_without_clicking_it() {
        let content = button("Task title")
            .on_press(Message::EventOpened("target".into()))
            .width(300)
            .height(50);
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(300.0, 100.0),
            task_drag(content, "target", Some("target")),
        );
        ui.point_at(Point::new(50.0, 20.0));
        ui.simulate([iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))]);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(&messages[..], [Message::TimeBlocking(Action::DropTask(id))] if id == "target")
        );
    }
}
