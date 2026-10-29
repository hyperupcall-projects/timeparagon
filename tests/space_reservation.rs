use time_slay::config::Config;

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
