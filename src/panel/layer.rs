//! The Wayland side panel is itself a layer surface, not a normal window
//! paired with a reservation helper. Translate the app's window tasks at this
//! boundary so its UI, IPC lifecycle, and background work remain shared.
use std::collections::BTreeMap;
use std::convert::Infallible;

use iced::{Element, Subscription, Task, window};
use iced_layershell::actions::{
    LayerShellCustomAction as LayerAction, LayerShellCustomActionWithId,
};
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_runtime::futures::futures::channel::oneshot;
use iced_runtime::{Action, window::Action as WindowAction};

use super::Side;
use crate::app::{self, State};
use crate::config::ViewSettings;

#[derive(Debug)]
enum Message {
    App(app::Message),
    Runtime(Action<app::Message>),
    Layer(LayerShellCustomActionWithId),
    Opened(window::Id, iced::Size),
}

impl TryFrom<Message> for LayerShellCustomActionWithId {
    type Error = Message;

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        match message {
            Message::Layer(action) => Ok(action),
            message => Err(message),
        }
    }
}

struct Panel {
    app: State,
    id: Option<window::Id>,
    width: u32,
    opening: BTreeMap<window::Id, oneshot::Sender<window::Id>>,
}

fn anchor(side: Side) -> Anchor {
    Anchor::Top
        | Anchor::Bottom
        | match side {
            Side::Left => Anchor::Left,
            Side::Right => Anchor::Right,
        }
}

fn exclusive_zone(views: &ViewSettings, width: u32) -> i32 {
    if views.panel_reserve_space {
        width.min(i32::MAX as u32) as i32
    } else {
        0
    }
}

fn layer(id: window::Id, action: LayerAction) -> Task<Message> {
    // New surfaces do not yet have a runtime window to address. Their new
    // iced ID belongs inside the action, not in its target-window field.
    let target = if matches!(action, LayerAction::NewLayerShell { .. }) {
        None
    } else {
        Some(id)
    };
    Task::done(Message::Layer(LayerShellCustomActionWithId::new(
        target, action,
    )))
}

/// Keep all async outputs and side effects, including chained tasks and their
/// response channels. The layer backend does not implement iced's Open/Resize
/// window actions, so these are handled here instead of silently dropped.
fn adapt(task: Task<app::Message>) -> Task<Message> {
    match iced_runtime::task::into_stream(task) {
        Some(stream) => Task::run(stream, Message::Runtime),
        None => Task::none(),
    }
}

fn update_app(panel: &mut Panel, message: app::Message) -> Task<Message> {
    let previous = panel.app.config.views.panel_reserve_space;
    let previous_width = panel.width;
    if let app::Message::WindowResized(id, size) = &message
        && Some(*id) == panel.id
    {
        panel.width = size.width.round().max(1.0) as u32;
    }
    let closed = match &message {
        app::Message::WindowClosed(id) => Some(*id),
        _ => None,
    };
    let task = app::update(&mut panel.app, message);
    if closed == panel.id {
        panel.id = None;
    }
    let reservation = if previous != panel.app.config.views.panel_reserve_space
        || previous_width != panel.width
    {
        panel
            .id
            .map(|id| {
                layer(
                    id,
                    LayerAction::ExclusiveZoneChange(exclusive_zone(
                        &panel.app.config.views,
                        panel.width,
                    )),
                )
            })
            .unwrap_or_else(Task::none)
    } else {
        Task::none()
    };
    Task::batch([adapt(task), reservation])
}

fn update(panel: &mut Panel, message: Message) -> Task<Message> {
    match message {
        Message::App(message) | Message::Runtime(Action::Output(message)) => {
            update_app(panel, message)
        }
        Message::Opened(id, size) => {
            if Some(id) == panel.id {
                panel.app.main_size = size;
                panel.width = size.width.round().max(1.0) as u32;
            }
            if let Some(sender) = panel.opening.remove(&id) {
                let _ = sender.send(id);
            }
            if Some(id) == panel.id {
                layer(
                    id,
                    LayerAction::ExclusiveZoneChange(exclusive_zone(
                        &panel.app.config.views,
                        panel.width,
                    )),
                )
            } else {
                Task::none()
            }
        }
        Message::Runtime(Action::Window(action)) => match action {
            WindowAction::Open(id, settings, sender) => {
                let flash = settings.fullscreen;
                let width = settings.size.width.round().max(1.0) as u32;
                let (anchor, size, zone, namespace) = if flash {
                    (
                        Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
                        (0, 0),
                        -1,
                        "timeslay-flash".to_owned(),
                    )
                } else {
                    panel.id = Some(id);
                    panel.width = width;
                    let side = panel
                        .app
                        .sidebar
                        .as_ref()
                        .map_or(Side::Left, |sidebar| sidebar.side);
                    (
                        anchor(side),
                        (width, 0),
                        exclusive_zone(&panel.app.config.views, width),
                        crate::window_effect::prepare(&panel.app.config.views, "timeslay-panel"),
                    )
                };
                panel.opening.insert(id, sender);
                layer(
                    id,
                    LayerAction::NewLayerShell {
                        id,
                        settings: NewLayerShellSettings {
                            size: Some(size),
                            layer: if flash { Layer::Overlay } else { Layer::Top },
                            anchor,
                            exclusive_zone: Some(zone),
                            margin: Some((0, 0, 0, 0)),
                            keyboard_interactivity: if flash {
                                KeyboardInteractivity::None
                            } else {
                                KeyboardInteractivity::OnDemand
                            },
                            output_option: OutputOption::LastOutput,
                            events_transparent: flash
                                && panel
                                    .app
                                    .flash
                                    .as_ref()
                                    .is_some_and(|f| f.settings.passthrough),
                            namespace: Some(namespace),
                        },
                    },
                )
            }
            WindowAction::Resize(id, size) if Some(id) == panel.id => {
                panel.width = size.width.round().max(1.0) as u32;
                // Height stays compositor-controlled between the top/bottom
                // anchors. Iced's transparent screen gutter is already part
                // of this width. Widget overlays never change it.
                layer(id, LayerAction::SizeChange((panel.width, 0))).chain(layer(
                    id,
                    LayerAction::ExclusiveZoneChange(exclusive_zone(
                        &panel.app.config.views,
                        panel.width,
                    )),
                ))
            }
            WindowAction::GetMonitorSize(_, sender) => {
                // Like winit on Wayland: there is no global monitor geometry.
                let _ = sender.send(None);
                Task::none()
            }
            WindowAction::SetMinSize(..)
            | WindowAction::SetMaxSize(..)
            | WindowAction::SetLevel(..)
            | WindowAction::GainFocus(..)
            | WindowAction::EnableMousePassthrough(..) => Task::none(),
            action => iced_runtime::task::effect(Action::<Infallible>::Window(action)),
        },
        Message::Runtime(action) => forward(action),
        Message::Layer(_) => unreachable!("layer actions are consumed by the runtime"),
    }
}

fn forward(action: Action<app::Message>) -> Task<Message> {
    let action = match action {
        Action::Output(_) | Action::Window(_) => unreachable!(),
        Action::LoadFont { bytes, channel } => Action::LoadFont { bytes, channel },
        Action::Widget(operation) => Action::Widget(operation),
        Action::Clipboard(action) => Action::Clipboard(action),
        Action::System(action) => Action::System(action),
        Action::Image(action) => Action::Image(action),
        Action::Reload => Action::Reload,
        Action::Exit => Action::Exit,
    };
    iced_runtime::task::effect(action)
}

fn boot() -> (Panel, Task<Message>) {
    let (app, task) = app::boot_sidebar();
    (
        Panel {
            app,
            id: None,
            width: 0,
            opening: BTreeMap::new(),
        },
        adapt(task),
    )
}

fn view(panel: &Panel, id: window::Id) -> Element<'_, Message> {
    app::view(&panel.app, id).map(Message::App)
}

fn subscription(panel: &Panel) -> Subscription<Message> {
    Subscription::batch([
        app::subscription(&panel.app).map(Message::App),
        iced::event::listen_with(|event, _, id| match event {
            iced::Event::Window(window::Event::Opened { size, .. }) => {
                Some(Message::Opened(id, size))
            }
            _ => None,
        }),
    ])
}

pub fn run() -> iced_layershell::Result {
    iced_layershell::daemon(boot, "timeslay-panel", update, view)
        .theme(|panel: &Panel, id| app::active_theme(&panel.app, id))
        .style(|_panel: &Panel, theme: &iced::Theme| {
            let mut style = iced::theme::default(theme);
            style.background_color = iced::Color::TRANSPARENT;
            style
        })
        .subscription(subscription)
        .settings(Settings {
            antialiasing: true,
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::{StreamExt, executor::block_on};

    fn outputs(task: Task<Message>) -> Vec<Message> {
        block_on(async {
            let Some(stream) = iced_runtime::task::into_stream(task) else {
                return Vec::new();
            };
            stream
                .filter_map(|action| async move {
                    match action {
                        Action::Output(message) => Some(message),
                        _ => None,
                    }
                })
                .collect()
                .await
        })
    }

    fn panel() -> Panel {
        let mut app = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        app.sidebar = Some(app::Sidebar {
            width: 200.0,
            side: Side::Left,
        });
        let id = window::Id::unique();
        app.main_window = Some(id);
        Panel {
            app,
            id: Some(id),
            width: 208,
            opening: BTreeMap::new(),
        }
    }

    #[test]
    fn panel_reservation_changes_live_without_changing_the_dock_setting() {
        let mut panel = panel();
        let id = panel.id.unwrap();
        for (enabled, expected) in [(true, 208), (false, 0)] {
            let messages = outputs(update_app(
                &mut panel,
                app::Message::PanelReserveSpaceToggled(enabled),
            ));
            assert!(messages.iter().any(|message| matches!(message,
                Message::Layer(LayerShellCustomActionWithId(Some(target), LayerAction::ExclusiveZoneChange(zone)))
                if *target == id && *zone == expected)));
            assert!(!panel.app.config.views.dock_reserve_space);
        }
    }

    #[test]
    fn new_panel_is_not_addressed_to_a_surface_that_does_not_exist_yet() {
        let mut panel = panel();
        panel.app.config.views.panel_reserve_space = true;
        let id = window::Id::unique();
        let (sender, _receiver) = oneshot::channel();
        let messages = outputs(update(
            &mut panel,
            Message::Runtime(Action::Window(WindowAction::Open(
                id,
                window::Settings {
                    size: iced::Size::new(208.0, 10_000.0),
                    ..Default::default()
                },
                sender,
            ))),
        ));
        assert!(
            matches!(messages.as_slice(), [Message::Layer(LayerShellCustomActionWithId(None,
            LayerAction::NewLayerShell { id: created, settings }))]
            if *created == id && settings.size == Some((208, 0)) && settings.exclusive_zone == Some(208))
        );
    }

    #[test]
    fn compositor_size_is_used_and_hiding_forgets_the_surface() {
        let mut panel = panel();
        panel.app.config.views.panel_reserve_space = true;
        let id = panel.id.unwrap();
        let messages = outputs(update(
            &mut panel,
            Message::Opened(id, iced::Size::new(190.0, 800.0)),
        ));
        assert_eq!(panel.width, 190);
        assert!(
            matches!(messages.as_slice(), [Message::Layer(LayerShellCustomActionWithId(Some(target), LayerAction::ExclusiveZoneChange(190)))] if *target == id)
        );
        panel.app.panel_expected_close = true;
        let _ = update_app(&mut panel, app::Message::WindowClosed(id));
        assert!(panel.id.is_none());
        assert!(panel.app.main_window.is_none());
        assert!(panel.app.config.views.panel_reserve_space);
    }
}
