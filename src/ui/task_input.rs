//! Title input with detected metadata ranges and cursor-aware Tab acceptance.
use crate::{app::Message, task_input::Token};
use iced::{Element, Length, Rectangle, Renderer, Size, Theme, mouse};
type InputState =
    iced::widget::text_input::State<<Renderer as iced::advanced::text::Renderer>::Paragraph>;
pub fn quick_input<'a>(
    content: impl Into<Element<'a, Message>>,
    value: &'a str,
    tokens: Vec<Token>,
    size: f32,
) -> Element<'a, Message> {
    Element::new(QuickInput {
        content: content.into(),
        value,
        tokens,
        size,
    })
}
struct QuickInput<'a> {
    content: Element<'a, Message>,
    value: &'a str,
    tokens: Vec<Token>,
    size: f32,
}
impl iced::advanced::widget::Widget<Message, Theme, Renderer> for QuickInput<'_> {
    fn children(&self) -> Vec<iced::advanced::widget::Tree> {
        vec![iced::advanced::widget::Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
        tree.diff_children(&[&self.content]);
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
        let state = tree.children[0].state.downcast_mut::<InputState>();
        if state.is_focused() {
            if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                modifiers,
                ..
            }) = event
            {
                if modifiers.is_empty() {
                    let value = iced::widget::text_input::Value::new(self.value);
                    if let iced::widget::text_input::cursor::State::Index(index) =
                        state.cursor().state(&value)
                    {
                        let byte = value.until(index).to_string().len();
                        if let Some(token) = self
                            .tokens
                            .iter()
                            .find(|t| t.range.start < byte && byte <= t.range.end)
                        {
                            let new_index = iced::widget::text_input::Value::new(
                                &self.value[..token.range.start],
                            )
                            .len();
                            state.move_cursor_to(new_index);
                            shell.publish(Message::Inbox(crate::inbox::Action::AcceptToken(
                                token.range.clone(),
                            )));
                            shell.capture_event();
                            return;
                        }
                    }
                }
            }
        }
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
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        use iced::advanced::{
            Renderer as _,
            text::{Paragraph as _, Renderer as _},
        };
        let bounds = layout.children().next().unwrap().bounds();
        let paragraph = <iced::Renderer as iced::advanced::text::Renderer>::Paragraph::with_text(
            iced::advanced::Text {
                content: self.value,
                bounds: iced::Size::new(f32::INFINITY, bounds.height),
                size: self.size.into(),
                line_height: iced::advanced::text::LineHeight::default(),
                font: renderer.default_font(),
                align_x: iced::advanced::text::Alignment::Default,
                align_y: iced::alignment::Vertical::Center,
                shaping: iced::advanced::text::Shaping::Advanced,
                wrapping: iced::advanced::text::Wrapping::None,
            },
        );
        let value = iced::widget::text_input::Value::new(self.value);
        let state = tree.children[0].state.downcast_ref::<InputState>();
        let index = match state.cursor().state(&value) {
            iced::widget::text_input::cursor::State::Index(i) => i,
            iced::widget::text_input::cursor::State::Selection { end, .. } => end,
        };
        let position = |i| paragraph.grapheme_position(0, i).unwrap_or_default().x;
        let offset = if state.is_focused() {
            (position(index) + 5.0 - bounds.width).max(0.0)
        } else {
            0.0
        };
        if let Some(clip) = bounds.intersection(viewport) {
            renderer.with_layer(clip, |renderer| {
                for token in &self.tokens {
                    let start =
                        iced::widget::text_input::Value::new(&self.value[..token.range.start])
                            .len();
                    let end =
                        iced::widget::text_input::Value::new(&self.value[..token.range.end]).len();
                    renderer.fill_quad(
                        iced::advanced::renderer::Quad {
                            bounds: Rectangle {
                                x: bounds.x + position(start) - offset,
                                y: bounds.y,
                                width: position(end) - position(start),
                                height: bounds.height,
                            },
                            border: iced::Border::default().rounded(3),
                            ..Default::default()
                        },
                        theme
                            .extended_palette()
                            .primary
                            .base
                            .color
                            .scale_alpha(0.25),
                    );
                }
            });
        }
    }

    fn mouse_interaction(
        &self,
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{
        Size,
        keyboard::key::Named,
        widget::{Id, text_input},
    };
    use iced_test::{Simulator, core::Settings};

    #[test]
    fn tab_accepts_only_the_highlight_at_the_cursor_and_enter_submits() {
        for (title, accept) in [("Café {oct 8}", true), ("Café {oct 8} ", false)] {
            let tokens = crate::task_input::detect(
                title,
                chrono::Local::now().date_naive(),
                &Default::default(),
            );
            let range = tokens[0].range.clone();
            let input = quick_input(
                text_input("Task", title)
                    .id(Id::new("test-title"))
                    .on_input(|v| Message::Inbox(crate::inbox::Action::Title(v)))
                    .on_submit(Message::Inbox(crate::inbox::Action::Create)),
                title,
                tokens,
                20.0,
            );
            let mut ui = Simulator::with_size(Settings::default(), Size::new(300.0, 60.0), input);
            ui.click(Id::new("test-title")).unwrap();
            ui.tap_key(Named::End);
            ui.tap_key(Named::Tab);
            ui.tap_key(Named::Enter);
            let messages: Vec<_> = ui.into_messages().collect();
            assert_eq!(messages.iter().any(|m| matches!(m, Message::Inbox(crate::inbox::Action::AcceptToken(r)) if *r == range)), accept);
            assert!(
                messages
                    .iter()
                    .any(|m| matches!(m, Message::Inbox(crate::inbox::Action::Create)))
            );
        }
    }
}
