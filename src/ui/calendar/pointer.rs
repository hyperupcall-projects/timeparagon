//! Where the pointer last was over the day view.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct PointerSample {
    pub(super) y: f32,
    pub(super) height: f32,
}

thread_local! {
    pub(super) static ROOT_BOUNDS: std::cell::Cell<Rectangle> = const { std::cell::Cell::new(Rectangle::new(Point::ORIGIN, Size::ZERO)) };
    pub(super) static CREATE_ANCHOR: std::cell::Cell<Rectangle> = const { std::cell::Cell::new(Rectangle::new(Point::ORIGIN, Size::ZERO)) };
    /// The last cursor position seen inside the day view. Kept per-thread so
    /// it needs no locking and never leaks across a test's simulations.
    pub(super) static SAMPLE: std::cell::Cell<PointerSample> = const {
        std::cell::Cell::new(PointerSample {
            y: 0.0,
            height: 0.0,
        })
    };
}

pub(crate) fn record_pointer(y: f32, height: f32) {
    SAMPLE.with(|sample| sample.set(PointerSample { y, height }));
}

/// True when the latest click landed in the top half of the day view.
pub fn click_was_in_the_top_half() -> bool {
    SAMPLE.with(|sample| {
        let sample = sample.get();
        sample.height <= 0.0 || sample.y <= sample.height / 2.0
    })
}

/// Remembers the cursor inside the day view without sending a message.
///
/// A message on every move would rebuild the grid while the pointer travels.
/// The click handler reads the sample after this widget has stored it.
pub(super) fn pointer_watch<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(PointerWatch {
        content: content.into(),
    })
}

pub(super) struct PointerWatch<'a> {
    pub(super) content: Element<'a, Message>,
}

impl iced::advanced::widget::Widget<Message, Theme, Renderer> for PointerWatch<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<iced::advanced::widget::Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
        self.content.as_widget().diff(tree);
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
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            self.content
                .as_widget_mut()
                .operate(tree, layout, renderer, operation);
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
        ROOT_BOUNDS.with(|root| root.set(layout.bounds()));
        if let Some(point) = cursor.position_in(layout.bounds()) {
            record_pointer(point.y, layout.bounds().height);
        }

        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
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
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}
