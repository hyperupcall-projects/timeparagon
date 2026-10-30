use timeslay::config::Config;

#[test]
fn old_configs_keep_overlays_and_reservations_round_trip_independently() {
    let old: Config = toml::from_str("[views]\ndock_width = 12.0\n").unwrap();
    assert!(!old.views.dock_reserve_space);
    assert!(!old.views.panel_reserve_space);
    for dock in [false, true] {
        for panel in [false, true] {
            let mut config = old.clone();
            config.views.dock_reserve_space = dock;
            config.views.panel_reserve_space = panel;
            let saved = toml::to_string(&config).unwrap();
            let restored: Config = toml::from_str(&saved).unwrap();
            assert_eq!(restored.views.dock_reserve_space, dock);
            assert_eq!(restored.views.panel_reserve_space, panel);
            assert_eq!(restored.views.dock_width, 12.0);
        }
    }
}

#[test]
fn radii_and_reservation_shortcut_are_independent_and_persist() {
    use timeslay::{
        app::{self, Message, State},
        panel::Action,
        theme::Catalog,
    };
    let mut state = State::new(Config::default(), Catalog::built_in());
    let _ = app::update(&mut state, Message::DockBorderRadiusChanged(12.0));
    let _ = app::update(&mut state, Message::PanelBorderRadiusChanged(7.0));
    state
        .config
        .panel
        .set_shortcuts(Action::ToggleReservation, vec!["Meta+Ctrl+U".into()]);
    let restored: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
    assert_eq!(restored.views.dock_border_radius(), 12.0);
    assert_eq!(restored.views.panel_border_radius(), 7.0);
    assert_eq!(
        restored.panel.shortcuts(Action::ToggleReservation),
        ["Meta+Ctrl+U"]
    );
    let old: Config = toml::from_str("[views]\nborder_radius = 9.0").unwrap();
    assert_eq!(old.views.panel_border_radius(), 9.0);
    assert_eq!(old.views.dock_border_radius(), 0.0);
}

#[test]
fn shortcut_command_toggles_only_panel_reservation() {
    use timeslay::{
        app::{self, Message, PanelCommand, State},
        theme::Catalog,
    };
    let mut state = State::new(Config::default(), Catalog::built_in());
    let (tx, rx) = std::sync::mpsc::channel();
    state.panel_command_rx = Some(rx);
    for enabled in [true, false] {
        tx.send(PanelCommand::ToggleReservation).unwrap();
        let _ = app::update(&mut state, Message::PanelCommandPolled);
        assert_eq!(state.config.views.panel_reserve_space, enabled);
        assert!(!state.config.views.dock_reserve_space);
    }
}
