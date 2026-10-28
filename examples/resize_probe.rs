//! Native window regression probe. Run on Wayland with `cargo run --example resize_probe`.
//! Exits nonzero if programmatic resizing leaves Iced's viewport at the old size.
use iced::widget::text;
use iced::{Element, Size, Subscription, Task, window};
use std::time::Duration;

#[derive(Default)]
struct Probe {
    id: Option<window::Id>,
    ticks: u32,
    expected: Option<Size>,
    checks: u8,
}
#[derive(Debug, Clone)]
enum Message {
    Opened(window::Id),
    Tick,
    Measured(Size),
    Captured(window::Screenshot),
}
fn boot() -> (Probe, Task<Message>) {
    let (_, open) = window::open(window::Settings {
        size: Size::new(480.0, 320.0),
        ..Default::default()
    });
    (Probe::default(), open.map(Message::Opened))
}
fn update(state: &mut Probe, message: Message) -> Task<Message> {
    match message {
        Message::Opened(id) => {
            state.id = Some(id);
        }
        Message::Tick => {
            if let Some(id) = state.id {
                state.ticks += 1;
                let requested = match state.ticks {
                    4 => Some(Size::new(880.0, 320.0)),
                    12 => Some(Size::new(480.0, 320.0)),
                    20 => Some(Size::new(880.0, 320.0)),
                    _ => None,
                };
                if let Some(size) = requested {
                    state.expected = Some(size);
                    eprintln!("requesting {size:?}");
                    return window::resize(id, size);
                }
                if [8, 16, 24].contains(&state.ticks) {
                    return window::size(id).map(Message::Measured);
                }
                if state.ticks == 28 {
                    assert_eq!(state.checks, 3);
                    return iced::exit();
                }
            }
        }
        Message::Measured(actual) => {
            let expected = state.expected.unwrap();
            eprintln!("viewport {actual:?}, expected {expected:?}");
            assert!(
                (actual.width - expected.width).abs() <= 1.0
                    && (actual.height - expected.height).abs() <= 1.0,
                "renderer viewport did not follow the native resize"
            );
            return window::screenshot(state.id.unwrap()).map(Message::Captured);
        }
        Message::Captured(screenshot) => {
            let expected = state.expected.unwrap();
            let logical = Size::new(
                screenshot.size.width as f32 / screenshot.scale_factor as f32,
                screenshot.size.height as f32 / screenshot.scale_factor as f32,
            );
            eprintln!("render buffer {logical:?} logical pixels");
            assert!(
                (logical.width - expected.width).abs() <= 1.0
                    && (logical.height - expected.height).abs() <= 1.0,
                "render buffer did not follow the native resize"
            );
            state.checks += 1;
        }
    }
    Task::none()
}
fn view(_state: &Probe, _id: window::Id) -> Element<'_, Message> {
    text("Resize probe: text should stay sharp while this window expands and contracts.")
        .size(18)
        .into()
}
fn main() -> iced::Result {
    iced::daemon(boot, update, view)
        .title("TimeSlay resize probe")
        .subscription(|_| -> Subscription<Message> {
            iced::time::every(Duration::from_millis(150)).map(|_| Message::Tick)
        })
        .run()
}
