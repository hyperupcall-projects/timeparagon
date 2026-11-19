//! Mobile application composition. Each service and screen owns its subscriptions.
mod authentication;
mod calendar_state;
mod calendar_view;
mod navigation;
mod polling;
mod settings;
mod sheets;
mod sleep_controls;
#[cfg(test)]
mod tests;

use chrono::NaiveDate;
use dioxus::prelude::*;
use timeparagon_core::calendar::first_day::FirstDayOfWeek;
use timeparagon_core::calendar::search::Search;
use timeparagon_core::calendar::{self, Event};
use timeparagon_core::config::CalendarSettings;
use timeparagon_core::more_menu::Item;

use crate::day::{DayModel, EditContext, Theme};
use crate::plan::{PlanView, TaskTab};
#[cfg(target_os = "android")]
use crate::widget;
use crate::{
	DayNav, Page, Pick, PlanNav, SCROLL_SCRIPT, SleepTimer, ViewTab, agenda, ai_input, alarms,
	calendar_data, calendar_gestures, create, css_color, day, editor, habits, linked_event,
	merged_log, month_picker, more, nav_icon, nav_tab, notes, people, reminders, schedule, scratch,
	search, setup, sleep, style, task_list, time_block_modal, timeline, travel, warnings,
};
use authentication::Auth;
use navigation::{Navigation, Panels, Selection};
use polling::{Session, SyncState};
use sleep_controls::SleepControl;

#[component]
pub(super) fn Main() -> Element {
	rsx! {
		polling::SessionProvider {
			navigation::NavigationProvider {
				authentication::AuthProvider {
					settings::SettingsProvider {
						sleep_controls::SleepProvider { Shell {} }
					}
				}
			}
		}
	}
}

/// Static composition: services and screens subscribe in their own scopes.
#[component]
fn Shell() -> Element {
	rsx! {
		document::Title { "TimeParagon · Day" }
		document::Style { {style::STYLESHEET.as_str()} }
		polling::PollLocalChanges {}
		polling::RefreshCalendar {}
		settings::ThemePreference {}
		div { class: "app",
			navigation::Topbar {}
			calendar_view::Workspace {}
		}
	}
}
