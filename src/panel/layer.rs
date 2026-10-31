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
    Focused(window::Id),
    Unfocused(window::Id),
    PopoverPolled,
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
    commands: Option<std::sync::mpsc::Receiver<PopoverCommand>>,
    created: Option<
        smithay_client_toolkit::reexports::calloop::channel::Sender<
            Result<crate::calendar::Event, String>,
        >,
    >,
}

enum PopoverCommand {
    Toggle,
    Stop,
}

/// A warm editor runtime owned by the dock, with no child process or per-click startup.
pub struct TaskPopover(std::sync::mpsc::Sender<PopoverCommand>);
impl TaskPopover {
    pub fn toggle(&self) {
        if self.0.send(PopoverCommand::Toggle).is_err() {
            log::error!("dock task editor runtime is no longer running");
        }
    }
}
impl Drop for TaskPopover {
    fn drop(&mut self) {
        let _ = self.0.send(PopoverCommand::Stop);
    }
}

pub fn spawn_task_popover(
    created: smithay_client_toolkit::reexports::calloop::channel::Sender<
        Result<crate::calendar::Event, String>,
    >,
) -> std::io::Result<TaskPopover> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("dock-task-editor".into())
        .spawn(move || {
            if let Err(error) = run_surface(true, Some((rx, created))) {
                log::error!("dock task editor stopped: {error}");
            }
        })?;
    Ok(TaskPopover(tx))
}

// Keep the compositor-provided geometry while hidden. A 1px height with
// top/bottom anchors and margins is invalid on KWin and closes the surface.
fn clear_input(id: window::Id) -> Task<Message> {
    layer(
        id,
        LayerAction::SetInputRegion(iced_layershell::actions::ActionCallback::new(|_| {})),
    )
}

fn show_popover(panel: &mut Panel) -> Task<Message> {
    log::debug!(
        "dock task editor toggle (surface={:?}, open={})",
        panel.id,
        panel.app.inbox.draft.is_some()
    );
    if panel.app.inbox.draft.is_some() {
        return update_app(panel, app::Message::Inbox(crate::inbox::Action::Cancel));
    }
    let edit = crate::inbox::update(&mut panel.app, crate::inbox::Action::NewTask);
    let show = panel
        .id
        .map(|id| {
            let height = panel.app.main_size.height.ceil().max(1.0) as i32;
            layer(
                id,
                LayerAction::SetInputRegion(iced_layershell::actions::ActionCallback::new(
                    move |region| region.add(0, 0, 440, height),
                )),
            )
            .chain(layer(
                id,
                LayerAction::KeyboardInteractivityChange(KeyboardInteractivity::Exclusive),
            ))
        })
        .unwrap_or_else(Task::none);
    Task::batch([adapt(edit), show])
}

fn anchor(side: Side) -> Anchor {
    Anchor::Top
        | Anchor::Bottom
        | match side {
            Side::Left => Anchor::Left,
            Side::Right => Anchor::Right,
        }
}

/// Place the shared task editor against the inner edge of the dock.
fn task_popover(views: &ViewSettings) -> (Anchor, (u32, u32), (i32, i32, i32, i32)) {
    use crate::config::DockSide;
    let dock = crate::dock::config::DockConfig::load(views);
    let gap = dock.popup_gap as i32;
    let inset = dock.screen_margin as i32;
    let thickness = dock.width as i32 + gap;
    // With zone 0 the compositor excludes desktop panels and the dock's own
    // reservation. Only add the remaining gap; adding its width again would
    // move the card away from the strip.
    let left = if dock.reserve_space {
        gap - dock.right_margin as i32
    } else {
        dock.left_margin as i32 + thickness
    };
    let right = if dock.reserve_space {
        gap - dock.left_margin as i32
    } else {
        dock.right_margin as i32 + thickness
    };
    let horizontal_edge = if dock.reserve_space { gap } else { thickness };
    match dock.side {
        DockSide::Left => (
            Anchor::Top | Anchor::Bottom | Anchor::Left,
            (440, 0),
            (inset, 0, inset, left),
        ),
        DockSide::Right => (
            Anchor::Top | Anchor::Bottom | Anchor::Right,
            (440, 0),
            (inset, right, inset, 0),
        ),
        DockSide::Top => (
            Anchor::Top | Anchor::Bottom | Anchor::Right,
            (440, 0),
            (horizontal_edge, dock.right_margin as i32, inset, 0),
        ),
        DockSide::Bottom => (
            Anchor::Top | Anchor::Bottom | Anchor::Right,
            (440, 0),
            (inset, dock.right_margin as i32, horizontal_edge, 0),
        ),
    }
}

fn editor_window_margin(panel: &Panel) -> (i32, i32, i32, i32) {
    let inset = panel.app.screen_margin().round() as i32;
    let top = crate::ui::editor_window_top(&panel.app).round() as i32;
    // The panel already contains a trailing gutter equal to `inset`.
    // Start at the surface edge to keep that same gap beside the editor.
    // Zone 0 already accounts for the panel when it reserves space.
    let offset = if panel.app.config.views.panel_reserve_space {
        0
    } else {
        panel.width as i32
    };
    match panel.app.sidebar.as_ref().map_or(Side::Left, |s| s.side) {
        Side::Left => (top, 0, inset, offset),
        Side::Right => (top, offset, inset, 0),
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
    let previous_top = crate::ui::editor_window_top(&panel.app);
    if let app::Message::WindowResized(id, size) = &message
        && Some(*id) == panel.id
    {
        panel.width = size.width.round().max(1.0) as u32;
    }
    let closed = match &message {
        app::Message::WindowClosed(id) => Some(*id),
        _ => None,
    };
    let was_open = panel.app.inbox.draft.is_some();
    let task = if panel.commands.is_some() {
        if let app::Message::Inbox(action) = message {
            if let crate::inbox::Action::Created(Ok(event)) = &action {
                if let Some(created) = &panel.created {
                    let _ = created.send(Ok(event.clone()));
                }
            }
            crate::inbox::update(&mut panel.app, action)
        } else {
            app::update(&mut panel.app, message)
        }
    } else {
        app::update(&mut panel.app, message)
    };
    let hide = if panel.commands.is_some() && was_open && panel.app.inbox.draft.is_none() {
        panel
            .id
            .map(|id| {
                layer(
                    id,
                    LayerAction::KeyboardInteractivityChange(KeyboardInteractivity::None),
                )
                .chain(clear_input(id))
            })
            .unwrap_or_else(Task::none)
    } else {
        Task::none()
    };
    if closed == panel.id {
        panel.id = None;
    }
    let reservation = if !panel.app.task_creation_window
        && (previous != panel.app.config.views.panel_reserve_space || previous_width != panel.width)
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
    let placement = if previous != panel.app.config.views.panel_reserve_space
        || previous_width != panel.width
        || previous_top != crate::ui::editor_window_top(&panel.app)
    {
        panel
            .app
            .editor_window
            .map(|id| layer(id, LayerAction::MarginChange(editor_window_margin(panel))))
            .unwrap_or_else(Task::none)
    } else {
        Task::none()
    };
    Task::batch([adapt(task), reservation, hide, placement])
}

fn update(panel: &mut Panel, message: Message) -> Task<Message> {
    match message {
        Message::App(app::Message::TaskPopoverBounds(bounds))
        | Message::Runtime(Action::Output(app::Message::TaskPopoverBounds(bounds))) => {
            if let Some(id) = panel.id.filter(|_| panel.app.task_creation_window) {
                let x = bounds.x.floor() as i32;
                let y = bounds.y.floor() as i32;
                let width = bounds.width.ceil() as i32;
                let height = bounds.height.ceil() as i32;
                layer(
                    id,
                    LayerAction::SetInputRegion(iced_layershell::actions::ActionCallback::new(
                        move |region| region.add(x, y, width, height),
                    )),
                )
            } else {
                Task::none()
            }
        }
        Message::PopoverPolled => {
            let commands: Vec<_> = panel
                .commands
                .as_ref()
                .map(|rx| rx.try_iter().collect())
                .unwrap_or_default();
            let mut tasks = Vec::new();
            for command in commands {
                match command {
                    PopoverCommand::Toggle => tasks.push(show_popover(panel)),
                    PopoverCommand::Stop => return iced::exit(),
                }
            }
            Task::batch(tasks)
        }
        Message::Focused(id) => {
            // Use Exclusive only to acquire initial focus. Keeping the grab
            // prevents other windows (including the screenshot UI) from taking
            // focus. Match the dock editor by yielding once focus arrives.
            if (panel.app.task_creation_window && Some(id) == panel.id)
                || panel.app.editor_window == Some(id)
            {
                layer(
                    id,
                    LayerAction::KeyboardInteractivityChange(KeyboardInteractivity::OnDemand),
                )
            } else {
                Task::none()
            }
        }
        Message::Unfocused(id) => {
            if panel.app.task_creation_window && Some(id) == panel.id {
                update_app(panel, app::Message::Inbox(crate::inbox::Action::Cancel))
            } else {
                Task::none()
            }
        }
        Message::App(message) | Message::Runtime(Action::Output(message)) => {
            update_app(panel, message)
        }
        Message::Opened(id, size) => {
            if panel.app.task_creation_window {
                log::debug!("dock task editor configured: {size:?}");
            }
            if Some(id) == panel.id {
                panel.app.main_size = size;
                panel.width = size.width.round().max(1.0) as u32;
            }
            if let Some(sender) = panel.opening.remove(&id) {
                let _ = sender.send(id);
            }
            if Some(id) == panel.id && panel.commands.is_some() && panel.app.inbox.draft.is_none() {
                return clear_input(id);
            }
            if Some(id) == panel.id && !panel.app.task_creation_window {
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
                let popover = panel.app.task_creation_window && !flash;
                // The app ID carries the animation choice, so identify the
                // editor by its window ID rather than a fixed application ID.
                let editor_window = panel.app.editor_window == Some(id);
                let mut margin = (0, 0, 0, 0);
                let (anchor, size, zone, namespace) = if editor_window {
                    let side = panel.app.sidebar.as_ref().map_or(Side::Left, |s| s.side);
                    margin = editor_window_margin(panel);
                    (
                        anchor(side),
                        (width, 0),
                        0,
                        settings.platform_specific.application_id.clone(),
                    )
                } else if popover {
                    panel.id = Some(id);
                    let (anchor, size, margins) = task_popover(&panel.app.config.views);
                    margin = margins;
                    panel.width = size.0;
                    (anchor, size, 0, "timeslay-task-popover".to_owned())
                } else if flash {
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
                            // Ordinary editors stay above apps, below system UI
                            // such as the screenshot selector and its dimming.
                            layer: if flash || popover {
                                Layer::Overlay
                            } else {
                                Layer::Top
                            },
                            anchor,
                            exclusive_zone: Some(zone),
                            margin: Some(margin),
                            keyboard_interactivity: if flash {
                                KeyboardInteractivity::None
                            } else if editor_window || (popover && panel.app.inbox.draft.is_some())
                            {
                                KeyboardInteractivity::Exclusive
                            } else {
                                if popover {
                                    KeyboardInteractivity::None
                                } else {
                                    KeyboardInteractivity::OnDemand
                                }
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
            WindowAction::Resize(id, size) if panel.app.editor_window == Some(id) => {
                let margin = editor_window_margin(panel);
                layer(id, LayerAction::SizeChange((size.width.ceil() as u32, 0)))
                    .chain(layer(id, LayerAction::MarginChange(margin)))
            }
            WindowAction::Resize(id, size) if Some(id) == panel.id => {
                if panel.app.task_creation_window {
                    return Task::none();
                }
                let previous_width = panel.width;
                panel.width = size.width.round().max(1.0) as u32;
                // Height stays compositor-controlled between the top/bottom
                // anchors. Iced's transparent screen gutter is already part
                // of this width. Widget overlays never change it.
                let resize = layer(id, LayerAction::SizeChange((panel.width, 0)));
                let reserve = layer(
                    id,
                    LayerAction::ExclusiveZoneChange(exclusive_zone(
                        &panel.app.config.views,
                        panel.width,
                    )),
                );
                // Release the old work area before shrinking the surface.
                // When growing, establish the larger surface before reserving it.
                if panel.width < previous_width {
                    reserve.chain(resize)
                } else {
                    resize.chain(reserve)
                }
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
    boot_surface(false)
}

fn boot_task_popover() -> (Panel, Task<Message>) {
    boot_surface(true)
}

fn boot_surface(popover: bool) -> (Panel, Task<Message>) {
    let (app, task) = if popover {
        app::boot_new_task()
    } else {
        app::boot_sidebar()
    };
    (
        Panel {
            app,
            id: None,
            width: 0,
            opening: BTreeMap::new(),
            commands: None,
            created: None,
        },
        adapt(task),
    )
}

fn view(panel: &Panel, id: window::Id) -> Element<'_, Message> {
    app::view(&panel.app, id).map(Message::App)
}

fn subscription(panel: &Panel) -> Subscription<Message> {
    let poll = if panel.commands.is_some() {
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::PopoverPolled)
    } else {
        Subscription::none()
    };
    Subscription::batch([
        poll,
        app::subscription(&panel.app).map(Message::App),
        iced::event::listen_with(|event, _, id| match event {
            iced::Event::Window(window::Event::Opened { size, .. }) => {
                Some(Message::Opened(id, size))
            }
            iced::Event::Window(window::Event::Focused) => Some(Message::Focused(id)),
            iced::Event::Window(window::Event::Unfocused) => Some(Message::Unfocused(id)),
            _ => None,
        }),
    ])
}

pub fn run() -> iced_layershell::Result {
    run_surface(false, None)
}

pub fn run_task_popover() -> iced_layershell::Result {
    run_surface(true, None)
}

type PopoverChannels = (
    std::sync::mpsc::Receiver<PopoverCommand>,
    smithay_client_toolkit::reexports::calloop::channel::Sender<
        Result<crate::calendar::Event, String>,
    >,
);
fn run_surface(popover: bool, channels: Option<PopoverChannels>) -> iced_layershell::Result {
    let channels = std::sync::Mutex::new(channels);
    iced_layershell::daemon(
        move || {
            let (mut panel, task) = if popover { boot_task_popover() } else { boot() };
            if let Some((rx, created)) = channels.lock().unwrap().take() {
                let _ = crate::inbox::update(&mut panel.app, crate::inbox::Action::Cancel);
                panel.commands = Some(rx);
                panel.created = Some(created);
            }
            (panel, task)
        },
        if popover {
            "timeslay-task-popover"
        } else {
            "timeslay-panel"
        },
        update,
        view,
    )
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
            width: 216,
            opening: BTreeMap::new(),
            commands: None,
            created: None,
        }
    }

    #[test]
    fn resident_editor_reuses_its_surface_when_shown_hidden_and_reopened() {
        let mut panel = panel();
        let id = panel.id.unwrap();
        panel.app.task_creation_window = true;
        let (_tx, rx) = std::sync::mpsc::channel();
        panel.commands = Some(rx);
        for _ in 0..2 {
            let messages = outputs(show_popover(&mut panel));
            assert!(panel.app.inbox.draft.is_some());
            assert_eq!(panel.id, Some(id));
            assert!(messages.iter().any(|m| matches!(m,
                Message::Layer(LayerShellCustomActionWithId(Some(target), LayerAction::SetInputRegion(_))) if *target == id)));
            assert!(!messages.iter().any(|m| matches!(
                m,
                Message::Layer(LayerShellCustomActionWithId(
                    _,
                    LayerAction::NewLayerShell { .. }
                ))
            )));
            let messages = outputs(update_app(
                &mut panel,
                app::Message::Inbox(crate::inbox::Action::Cancel),
            ));
            assert!(panel.app.inbox.draft.is_none());
            assert_eq!(panel.id, Some(id));
            assert!(messages.iter().any(|m| matches!(m,
                Message::Layer(LayerShellCustomActionWithId(Some(target), LayerAction::SetInputRegion(_))) if *target == id)));
        }
    }

    #[test]
    fn warm_hidden_editor_has_valid_geometry_with_screen_margins() {
        let mut panel = panel();
        panel.app.task_creation_window = true;
        let (_tx, rx) = std::sync::mpsc::channel();
        panel.commands = Some(rx);
        let id = window::Id::unique();
        let (sender, _receiver) = oneshot::channel();
        let messages = outputs(update(
            &mut panel,
            Message::Runtime(Action::Window(WindowAction::Open(
                id,
                window::Settings::default(),
                sender,
            ))),
        ));
        assert!(
            matches!(&messages[..], [Message::Layer(LayerShellCustomActionWithId(None,
            LayerAction::NewLayerShell { settings, .. }))]
            if settings.size == Some((440, 0)) && settings.keyboard_interactivity == KeyboardInteractivity::None)
        );
        let messages = outputs(update(
            &mut panel,
            Message::Opened(id, iced::Size::new(440.0, 900.0)),
        ));
        assert!(
            matches!(&messages[..], [Message::Layer(LayerShellCustomActionWithId(Some(target), LayerAction::SetInputRegion(_)))] if *target == id)
        );
        for _ in 0..2 {
            let _ = outputs(show_popover(&mut panel));
            let messages = outputs(show_popover(&mut panel));
            assert!(!messages.iter().any(|m| matches!(
                m,
                Message::Layer(LayerShellCustomActionWithId(_, LayerAction::SizeChange(_)))
            )));
        }
    }

    #[test]
    fn popover_offsets_account_for_the_docks_own_reserved_area() {
        for side in [
            crate::config::DockSide::Left,
            crate::config::DockSide::Right,
        ] {
            let mut views = ViewSettings::default();
            views.dock_side = side;
            views.dock_left_margin = Some(12.0);
            views.dock_right_margin = Some(18.0);
            let (_, _, unreserved) = task_popover(&views);
            views.dock_reserve_space = true;
            let (_, _, reserved) = task_popover(&views);
            let dock = crate::dock::config::DockConfig::load(&views);
            let occupied = dock.width as i32 + dock.left_margin as i32 + dock.right_margin as i32;
            assert_eq!(reserved.0, unreserved.0);
            assert_eq!(reserved.2, unreserved.2);
            if side == crate::config::DockSide::Left {
                assert_eq!(reserved.3 + occupied, unreserved.3);
            } else {
                assert_eq!(reserved.1 + occupied, unreserved.1);
            }
        }
    }

    #[test]
    fn dock_task_editor_is_an_anchored_overlay_without_a_reservation() {
        for side in [
            crate::config::DockSide::Left,
            crate::config::DockSide::Right,
            crate::config::DockSide::Top,
            crate::config::DockSide::Bottom,
        ] {
            let mut panel = panel();
            panel.app.task_creation_window = true;
            panel.app.config.views.dock_side = side;
            panel.app.config.views.panel_reserve_space = true;
            let id = window::Id::unique();
            let (sender, _receiver) = oneshot::channel();
            let expected = task_popover(&panel.app.config.views);
            let messages = outputs(update(
                &mut panel,
                Message::Runtime(Action::Window(WindowAction::Open(
                    id,
                    window::Settings::default(),
                    sender,
                ))),
            ));
            assert!(matches!(&messages[..],
                [Message::Layer(LayerShellCustomActionWithId(None,
                    LayerAction::NewLayerShell { id: target, settings }))]
                if *target == id && settings.anchor == expected.0
                    && settings.size == Some(expected.1) && settings.margin == Some(expected.2)
                    && settings.exclusive_zone == Some(0) && settings.layer == Layer::Overlay
                    && settings.namespace.as_deref() == Some("timeslay-task-popover")));
            assert!(
                outputs(update(
                    &mut panel,
                    Message::Opened(id, iced::Size::new(440.0, 800.0))
                ))
                .is_empty()
            );
            if side == crate::config::DockSide::Left {
                assert!(expected.0.contains(Anchor::Left));
                assert!(expected.2.3 > panel.app.config.views.dock_width() as i32);
            }
        }
    }

    #[test]
    fn editor_window_yields_keyboard_focus_like_the_dock_popover() {
        let mut panel = panel();
        let main = panel.id;
        let editor = window::Id::unique();
        panel.app.editor_window = Some(editor);
        for _ in 0..2 {
            let messages = outputs(update(&mut panel, Message::Focused(editor)));
            assert!(matches!(messages.as_slice(),
                [Message::Layer(LayerShellCustomActionWithId(Some(target),
                    LayerAction::KeyboardInteractivityChange(KeyboardInteractivity::OnDemand)))]
                if *target == editor));
            // Switching to another app or the screenshot tool leaves the
            // editor open, without taking focus back or changing geometry.
            assert!(outputs(update(&mut panel, Message::Unfocused(editor))).is_empty());
            assert_eq!(panel.app.editor_window, Some(editor));
            assert_eq!(panel.id, main);
            assert_eq!(panel.width, 216);
        }
        assert!(outputs(update(&mut panel, Message::Focused(main.unwrap()))).is_empty());
        panel.app.editor_window = None;
        assert!(outputs(update(&mut panel, Message::Focused(editor))).is_empty());
    }

    #[test]
    fn editor_window_floats_beside_the_panel_without_changing_its_reservation() {
        for side in [Side::Left, Side::Right] {
            for reserved in [false, true] {
                for mode in crate::config::WindowAnimation::ALL {
                    let application_id =
                        crate::window_effect::application_id(mode, 240, "timeslay-editor");
                    let mut panel = panel();
                    panel.app.sidebar.as_mut().unwrap().side = side;
                    panel.app.config.views.panel_reserve_space = reserved;
                    let main = panel.id;
                    let id = window::Id::unique();
                    panel.app.editor_window = Some(id);
                    let (sender, _receiver) = oneshot::channel();
                    let messages = outputs(update(
                        &mut panel,
                        Message::Runtime(Action::Window(WindowAction::Open(
                            id,
                            window::Settings {
                                size: iced::Size::new(360.0, 800.0),
                                platform_specific: window::settings::PlatformSpecific {
                                    application_id: application_id.clone(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                            sender,
                        ))),
                    ));
                    assert_eq!(panel.id, main);
                    assert_eq!(panel.width, 216);
                    assert!(matches!(messages.as_slice(),
                        [Message::Layer(LayerShellCustomActionWithId(None, LayerAction::NewLayerShell { id: target, settings }))]
                        if *target == id && settings.layer == Layer::Top
                            && settings.namespace.as_deref() == Some(application_id.as_str())
                            && settings.exclusive_zone == Some(0)
                            && settings.anchor == anchor(side)
                            && settings.size == Some((360, 0))
                            && settings.keyboard_interactivity == KeyboardInteractivity::Exclusive
                            && settings.margin.is_some_and(|m| match side {
                                Side::Left => m.3 == (if reserved { 0 } else { 216 }) && m.1 == 0
                                    && m.0 == 8 && m.2 == 8,
                                Side::Right => m.1 == (if reserved { 0 } else { 216 }) && m.3 == 0
                                    && m.0 == 8 && m.2 == 8,
                            })
                    ));
                }
            }
        }
    }

    #[test]
    fn panel_reservation_changes_live_without_changing_the_dock_setting() {
        let mut panel = panel();
        let id = panel.id.unwrap();
        for (enabled, expected) in [(true, 216), (false, 0)] {
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
    fn margin_changes_resize_and_reserve_both_gutters_in_both_directions() {
        for side in [Side::Left, Side::Right] {
            let mut panel = panel();
            panel.app.sidebar.as_mut().unwrap().side = side;
            panel.app.config.views.panel_reserve_space = true;
            for margin in [20.0, 3.0, 0.0, 30.0, 8.0] {
                let expected = 200 + (margin * 2.0) as u32;
                let mut pending = std::collections::VecDeque::from(outputs(update_app(
                    &mut panel,
                    app::Message::ScreenMarginChanged(margin),
                )));
                let mut reserved = None;
                let mut resized = None;
                while let Some(message) = pending.pop_front() {
                    match message {
                        Message::Layer(LayerShellCustomActionWithId(
                            _,
                            LayerAction::ExclusiveZoneChange(zone),
                        )) => reserved = Some(zone),
                        Message::Layer(LayerShellCustomActionWithId(
                            _,
                            LayerAction::SizeChange((width, 0)),
                        )) => resized = Some(width),
                        message => pending.extend(outputs(update(&mut panel, message))),
                    }
                }
                assert_eq!(resized, Some(expected));
                assert_eq!(reserved, Some(expected as i32));
                assert_eq!(panel.width, expected);
            }
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
                    size: iced::Size::new(216.0, 10_000.0),
                    ..Default::default()
                },
                sender,
            ))),
        ));
        assert!(
            matches!(messages.as_slice(), [Message::Layer(LayerShellCustomActionWithId(None,
            LayerAction::NewLayerShell { id: created, settings }))]
            if *created == id && settings.size == Some((216, 0)) && settings.exclusive_zone == Some(216))
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
