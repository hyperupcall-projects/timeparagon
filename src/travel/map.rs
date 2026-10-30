//! Interactive place picker. CEF stays alive between visits (one lifecycle per
//! process); individual pages close on cancel/confirm. Title messages carry a
//! session id so delayed browser callbacks cannot select a place in a new picker.
use super::{Found, PickTarget};
use crate::app::{Message, State};
use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Subscription, Task};
use iced_webview::{Action as WebAction, Cef, PageType, WebView};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub enum Action {
    Open,
    Close,
    Confirm,
    Browser(WebAction),
    Title(String),
}

#[derive(Default)]
pub struct Workspace {
    browser: Option<WebView<Cef, Message>>,
    pub active: bool,
    session: u64,
    target: Option<PickTarget>,
    pub selected: Option<Found>,
    error: Option<String>,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapPicker")
            .field("active", &self.active)
            .finish()
    }
}

fn msg(action: Action) -> Message {
    Message::Travel(super::Action::Map(action))
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::Open => {
            let Some(picker) = state.travel.picker.as_ref() else {
                return Task::none();
            };
            let Some(key) = state.config.travel.key() else {
                state.travel.picker.as_mut().unwrap().error = Some(super::NO_KEY.into());
                return Task::none();
            };
            let target = picker.target.clone();
            let query = picker.query.clone();
            let home = state
                .config
                .travel
                .home
                .and_then(|id| state.config.travel.place(id));
            let center = home.map(|p| [p.lng, p.lat]);
            let map = &mut state.travel.map;
            if map.active {
                return Task::none();
            }
            map.session += 1;
            map.active = true;
            map.target = Some(target);
            map.selected = None;
            map.error = None;
            if map.browser.is_none() {
                match Cef::try_new() {
                    Ok(engine) => {
                        map.browser = Some(
                            WebView::with_engine(engine)
                                .with_initial_size(iced::Size::new(800, 600))
                                .on_title_change(|title| msg(Action::Title(title)))
                                .on_action(|action| msg(Action::Browser(action))),
                        )
                    }
                    Err(error) => {
                        map.error = Some(format!("Could not open the map: {error}"));
                        return Task::none();
                    }
                }
            }
            let html = page(&key, &query, center, map.session);
            let browser = map.browser.as_mut().unwrap();
            let create = browser.update(WebAction::CreateView(PageType::Html(html)));
            let show = browser.update(WebAction::ChangeView(0));
            Task::batch([create, show])
        }
        Action::Close => close(&mut state.travel.map),
        Action::Confirm => {
            let map = &mut state.travel.map;
            if !map.active {
                return Task::none();
            }
            let (Some(found), Some(target)) = (map.selected.take(), map.target.clone()) else {
                return Task::none();
            };
            // The underlying field must still be the same one that opened us.
            if state.travel.picker.as_ref().map(|p| &p.target) != Some(&target) {
                return close(map);
            }
            let id = state.config.travel.add_place(
                &found.name,
                &found.address,
                &found.place_id,
                found.lat,
                found.lng,
            );
            let closed = close(&mut state.travel.map);
            state.travel.picker = None;
            Task::batch([closed, super::assign(state, &target, Some(id))])
        }
        Action::Browser(action) => {
            let map = &mut state.travel.map;
            if !map.active && !matches!(action, WebAction::Update) {
                return Task::none();
            }
            map.browser
                .as_mut()
                .map_or_else(Task::none, |browser| browser.update(action))
        }
        Action::Title(title) => {
            let map = &mut state.travel.map;
            if map.active
                && let Some(message) = bridge_message(&title, map.session)
            {
                map.selected = message.selected;
                map.error = message.error;
            }
            Task::none()
        }
    }
}

fn close(map: &mut Workspace) -> Task<Message> {
    map.active = false;
    map.selected = None;
    map.target = None;
    map.error = None;
    map.browser.as_mut().map_or_else(Task::none, |browser| {
        browser.update(WebAction::CloseCurrentView)
    })
}

#[derive(Deserialize)]
struct BridgeMessage {
    session: u64,
    selected: Option<Found>,
    error: Option<String>,
}

fn bridge_message(title: &str, session: u64) -> Option<BridgeMessage> {
    let message: BridgeMessage = serde_json::from_str(title.strip_prefix("timeslay-map:")?).ok()?;
    if message.session != session {
        return None;
    }
    if let Some(place) = &message.selected {
        if place.place_id.trim().is_empty()
            || place.name.trim().is_empty()
            || !place.lng.is_finite()
            || !place.lat.is_finite()
            || !(-180.0..=180.0).contains(&place.lng)
            || !(-90.0..=90.0).contains(&place.lat)
        {
            return None;
        }
    }
    Some(message)
}

fn page(key: &str, query: &str, center: Option<[f64; 2]>, session: u64) -> String {
    let config =
        serde_json::json!({"token": key, "query": query, "center": center, "session": session});
    // Script-safe JSON: a query/address must never terminate the script element.
    let json = config
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    include_str!("../../assets/map-picker.html").replace("/*TIMESLAY_CONFIG*/null", &json)
}

pub fn subscription(state: &State) -> Subscription<Message> {
    if state.travel.map.browser.is_some() {
        // Keep Chromium's loop alive to finish asynchronous browser closes.
        let interval = if state.travel.map.active { 16 } else { 100 };
        iced::time::every(std::time::Duration::from_millis(interval))
            .map(|_| msg(Action::Browser(WebAction::Update)))
    } else {
        Subscription::none()
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let map = &state.travel.map;
    let mut content = column![
        text("Choose a location").size(22),
        text("Search for an address or place, or click the map. Drag to move and scroll to zoom.")
            .size(14),
    ]
    .spacing(10)
    .width(Fill)
    .height(Fill);
    if let Some(browser) = &map.browser {
        content = content.push(
            container(browser.view().map(|action| msg(Action::Browser(action))))
                .width(Fill)
                .height(Fill),
        );
    }
    if let Some(error) = &map.error {
        content = content.push(text(error).style(text::danger));
    }
    if let Some(found) = &map.selected {
        content = content
            .push(text(&found.name).size(16))
            .push(text(&found.address).size(13));
    } else {
        content = content.push(text("Select a location to confirm").size(14));
    }
    let confirm = button(text("Confirm").center().width(Fill))
        .width(Fill)
        .style(crate::ui::controls::primary);
    let confirm = if map.selected.is_some() {
        confirm.on_press(msg(Action::Confirm))
    } else {
        confirm
    };
    content = content.push(
        row![
            button(text("Back to search").center().width(Fill))
                .width(Fill)
                .on_press(msg(Action::Close))
                .style(crate::ui::controls::secondary),
            confirm,
        ]
        .spacing(10),
    );
    container(content)
        .padding(16)
        .width(Fill)
        .height(Fill)
        .style(crate::ui::controls::surface)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        )
    }
    #[test]
    fn confirm_requires_a_selection_and_saves_to_the_original_field() {
        let mut state = state();
        let _ = super::super::update(
            &mut state,
            super::super::Action::OpenPicker(PickTarget::Home),
        );
        state.travel.map.active = true;
        state.travel.map.target = Some(PickTarget::Home);
        let _ = update(&mut state, Action::Confirm);
        assert!(state.travel.map.active);
        assert!(state.config.travel.home.is_none());
        state.travel.map.selected = Some(Found {
            place_id: "cafe".into(),
            name: "Cafe".into(),
            address: "1 Main St".into(),
            lng: -122.4,
            lat: 37.8,
        });
        let _ = update(&mut state, Action::Confirm);
        let home = state
            .config
            .travel
            .place(state.config.travel.home.unwrap())
            .unwrap();
        assert_eq!(home.address, "1 Main St");
        assert!(!state.travel.map.active);
        assert!(state.travel.picker.is_none());
    }
    #[test]
    fn cancel_preserves_home_and_search_text() {
        let mut state = state();
        let _ = super::super::update(
            &mut state,
            super::super::Action::OpenPicker(PickTarget::Home),
        );
        state.travel.picker.as_mut().unwrap().query = "coffee".into();
        state.travel.map.active = true;
        let _ = update(&mut state, Action::Close);
        assert!(!state.travel.map.active);
        assert_eq!(state.travel.picker.as_ref().unwrap().query, "coffee");
        assert!(state.config.travel.home.is_none());
    }
    #[test]
    fn page_config_cannot_inject_html() {
        let html = page(
            "pk.test",
            "</script><script>alert(1)</script>",
            Some([-122.4, 37.8]),
            9,
        );
        assert!(!html.contains("</script><script>alert"));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(!html.contains("/*TIMESLAY_CONFIG*/"));
    }
    #[test]
    fn bridge_rejects_old_sessions_and_invalid_places() {
        let title = r#"timeslay-map:{"session":9,"selected":{"place_id":"id","name":"Cafe","address":"1 Main St","lat":37.8,"lng":-122.4}}"#;
        assert!(bridge_message(title, 9).unwrap().selected.is_some());
        assert!(bridge_message(title, 10).is_none());
        assert!(bridge_message(&title.replace("37.8", "100"), 9).is_none());
        assert!(bridge_message("A normal page title", 9).is_none());
        assert!(
            bridge_message(r#"timeslay-map:{"session":9,"selected":null}"#, 9)
                .unwrap()
                .selected
                .is_none()
        );
    }
}
