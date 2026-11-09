//! Planning periods and the lists under Tasks: Overdue, Inbox, Today and
//! Backlog. The lists follow the same rules as the desktop's,
//! so a task shows in the same list on both.

use chrono::{DateTime, Local, NaiveDate, NaiveTime};
use dioxus::prelude::*;
use timeslay_core::calendar::task_sort::InboxSort;
use timeslay_core::calendar::time_format::{self, TimeFormat};
use timeslay_core::calendar::{self, Calendar, Event, deadline, ids};
use timeslay_core::config::{CalendarSettings, Config};
use timeslay_core::task_edit::Change;
use timeslay_core::timeblocking as tb;

use crate::agenda::Range;
use crate::css_color;
use crate::day::{self, EditContext};
use crate::editor::worker;

/// The period the Plan button plans, which it is named after.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlanView {
    Day,
    Week,
    TwoWeeks,
    Month,
}

impl PlanView {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::TwoWeeks, Self::Month];

    /// The Plan button's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "Plan Day",
            Self::Week => "Plan Week",
            Self::TwoWeeks => "Plan 2 Weeks",
            Self::Month => "Plan Month",
        }
    }

    /// The label in the Plan menu.
    pub fn menu_label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::TwoWeeks => "2 Weeks",
            Self::Month => "Month",
        }
    }

    /// The range drawn by Plan; a day uses the day calendar.
    pub fn range(self) -> Option<Range> {
        match self {
            Self::Day => None,
            Self::Week => Some(Range::Week),
            Self::TwoWeeks => Some(Range::TwoWeeks),
            Self::Month => Some(Range::Month),
        }
    }
}

/// The lists beneath the Tasks button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskTab {
    Overdue,
    Inbox,
    Today,
    Backlog,
}

impl TaskTab {
    pub const ALL: [Self; 4] = [Self::Overdue, Self::Inbox, Self::Today, Self::Backlog];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overdue => "Overdue",
            Self::Inbox => "Inbox",
            Self::Today => "Today",
            Self::Backlog => "Backlog",
        }
    }
}

/// Every task still to be triaged, in the chosen order.
pub fn inbox_tasks<'a>(
    events: &'a [Event],
    settings: &CalendarSettings,
    calendars: &[Calendar],
) -> Vec<&'a Event> {
    let mut tasks: Vec<_> = events
        .iter()
        .filter(|event| event.is_task() && day::untriaged_in(event, settings, calendars))
        .collect();
    sort_tasks(&mut tasks, settings.inbox_sort);
    tasks
}

/// Open date-only tasks for today that have no block or time slot yet.
pub fn today_tasks<'a>(
    events: &'a [Event],
    settings: &CalendarSettings,
    now: DateTime<Local>,
) -> Vec<&'a Event> {
    let mut tasks: Vec<_> = events
        .iter()
        .filter(|event| tb::unslotted_on(event, now.date_naive(), now))
        .collect();
    sort_tasks(&mut tasks, settings.inbox_sort);
    tasks
}

/// Open tasks on any list whose planned time or deadline has passed.
pub fn overdue_tasks<'a>(
    events: &'a [Event],
    settings: &CalendarSettings,
    now: DateTime<Local>,
) -> Vec<&'a Event> {
    let mut tasks: Vec<_> = events
        .iter()
        .filter(|event| tb::past_due(event, now))
        .collect();
    sort_tasks(&mut tasks, settings.overdue_sort);
    tasks
}

/// Whether `calendar_id` is the list that holds backlog tasks.
pub fn is_backlog_list(
    settings: &CalendarSettings,
    calendars: &[Calendar],
    calendar_id: &str,
) -> bool {
    if let Some(chosen) = &settings.backlog_list {
        return ids::bare_list(chosen) == ids::bare_list(calendar_id);
    }
    let default = CalendarSettings::DEFAULT_BACKLOG_LIST;
    calendar_id
        .trim_start_matches("tasks:")
        .eq_ignore_ascii_case(default)
        || calendars.iter().any(|calendar| {
            calendar.id == calendar_id
                && calendar.is_task_list()
                && calendar.name.trim().eq_ignore_ascii_case(default)
        })
}

/// The backlog list's name, from its title when it is known.
pub fn backlog_list_name(settings: &CalendarSettings, calendars: &[Calendar]) -> String {
    let id = settings
        .backlog_list
        .clone()
        .unwrap_or_else(|| format!("tasks:{}", CalendarSettings::DEFAULT_BACKLOG_LIST));
    calendars
        .iter()
        .find(|calendar| calendar.id == id)
        .map(|calendar| calendar.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            id.strip_prefix(ids::TASK_LIST)
                .filter(|id| !id.is_empty())
                .unwrap_or("Tasks")
                .to_owned()
        })
}

/// Open tasks parked on the backlog list, soonest due first.
pub fn backlog_tasks<'a>(
    events: &'a [Event],
    settings: &CalendarSettings,
    calendars: &[Calendar],
) -> Vec<&'a Event> {
    let mut tasks: Vec<_> = events
        .iter()
        .filter(|event| {
            event.is_task()
                && !event.completed
                && tb::metadata(event).block.is_none()
                && is_backlog_list(settings, calendars, &event.calendar_id)
        })
        .collect();
    tasks.sort_by_key(|event| backlog_order(event));
    tasks
}

/// When a task is due: its notes deadline if it has one, otherwise its day.
fn due(event: &Event) -> Option<deadline::Deadline> {
    if let Ok(Some(deadline)) = deadline::read_deadline(&event.details.description) {
        return Some(deadline);
    }
    let date = event.start.date_naive();
    (date != tb::UNDATED).then_some(deadline::Deadline { date, time: None })
}

/// Soonest due first, then later ones, then tasks with no due date. Ties keep
/// the manual order.
fn backlog_order(event: &Event) -> (bool, Option<NaiveDate>, Option<NaiveTime>, i64) {
    let due = due(event);
    (
        due.is_none(),
        due.map(|due| due.date),
        due.and_then(|due| due.time),
        tb::metadata(event).order,
    )
}

fn sort_tasks(tasks: &mut [&Event], sort: InboxSort) {
    tasks.sort_by_cached_key(|event| {
        let metadata = tb::metadata(event);
        let timestamp = match sort {
            InboxSort::CreatedAscending | InboxSort::CreatedDescending => metadata.created_at,
            InboxSort::ModifiedAscending | InboxSort::ModifiedDescending => {
                event.details.task_updated_at
            }
            InboxSort::Manual => None,
        };
        let descending = matches!(
            sort,
            InboxSort::CreatedDescending | InboxSort::ModifiedDescending
        );
        let manual = if sort == InboxSort::Manual {
            metadata.order
        } else {
            0
        };
        // Unknown timestamps go last in either direction, and ties stay put
        // across syncs.
        (
            timestamp.is_none(),
            timestamp.filter(|_| !descending),
            std::cmp::Reverse(timestamp.filter(|_| descending)),
            manual,
            event.title.to_lowercase(),
            event.id.clone(),
        )
    });
}

/// The day or time a task has been planned for, if it has one.
fn scheduled_label(task: &Event, format: TimeFormat) -> Option<String> {
    let date = task.start.date_naive();
    if date == tb::UNDATED {
        return None;
    }
    Some(if task.all_day {
        date.format("%a, %b %-d").to_string()
    } else {
        format!(
            "{} at {}",
            date.format("%a, %b %-d"),
            time_format::clock(task.start, format)
        )
    })
}

/// "1h 30m", "45m" or "2h".
fn duration(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

fn timestamp(time: Option<DateTime<chrono::Utc>>, format: TimeFormat) -> String {
    let Some(time) = time else {
        return "Unknown".to_owned();
    };
    let local = time.with_timezone(&Local);
    format!(
        "{} {}",
        local.format("%Y-%m-%d"),
        time_format::clock(local, format)
    )
}

/// One of Overdue, Inbox, Today or Backlog.
#[component]
pub fn TaskList(
    tab: TaskTab,
    config: Signal<Config>,
    /// What the lists choose from: every task loaded, with local edits.
    events: Vec<Event>,
    calendars: Vec<Calendar>,
    /// What a write lays out around; absent until a day has loaded.
    context: Option<EditContext>,
    signed_in: bool,
    /// Bumped by every write, so invitations are asked for again.
    revision: Signal<u32>,
    editing: Signal<Option<Event>>,
    on_new_task: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let mut sort_error = use_signal(|| None::<String>);
    let settings = config.read().calendar.clone();
    let now = timeslay_core::clock::now();
    let (heading, subtitle, tasks): (String, &str, Vec<&Event>) = match tab {
        TaskTab::Overdue => (
            "Overdue".to_owned(),
            "Incomplete tasks with a past scheduled time or deadline. This should be empty! Move tasks that you plan to do in inbox, schedule them for later, or move them to the backlog.",
            overdue_tasks(&events, &settings, now),
        ),
        TaskTab::Inbox => (
            "Inbox".to_owned(),
            "Tasks waiting to be triaged",
            inbox_tasks(&events, &settings, &calendars),
        ),
        TaskTab::Today => (
            "Today".to_owned(),
            "Tasks for today that still need a time slot",
            today_tasks(&events, &settings, now),
        ),
        TaskTab::Backlog => {
            return rsx! {
                Backlog {
                    config,
                    events: events.clone(),
                    calendars: calendars.clone(),
                    context,
                    editing,
                    on_saved,
                }
            };
        }
    };

    let invitations = if tab == TaskTab::Inbox && signed_in {
        rsx! { Invitations { calendars: calendars.clone(), settings: settings.clone(), revision, editing } }
    } else {
        rsx! {}
    };
    let sort = match tab {
        TaskTab::Overdue => Some(settings.overdue_sort),
        TaskTab::Inbox => Some(settings.inbox_sort),
        _ => None,
    };
    let count = tasks.len();
    let empty = match tab {
        TaskTab::Overdue => "Nothing overdue",
        TaskTab::Inbox => "Nothing to triage",
        _ => "No tasks left to slot in today",
    };
    let time_format = config.read().appearance.time_format;
    let cards: Vec<Element> = tasks
        .into_iter()
        .map(|task| task_card(task, tab, &config.read(), time_format, editing))
        .collect();

    rsx! {
        div { class: "plan-list", id: "plan-list",
            div { class: "plan-head",
                div { class: "plan-titles",
                    h2 { class: "plan-title",
                        "{heading}"
                        span { class: "count-pill", "{count}" }
                    }
                    p { class: "hint plan-subtitle", "{subtitle}" }
                }
                if tab == TaskTab::Inbox {
                    button { class: "btn primary", onclick: move |_| on_new_task.call(()), "New task" }
                }
            }
            if let Some(chosen) = sort {
                label { class: "plan-sort",
                    span { "Sort by" }
                    select {
                        class: "client-input",
                        onchange: move |event: FormEvent| {
                            let Some(picked) = InboxSort::ALL
                                .into_iter()
                                .find(|sort| sort.name() == event.value())
                            else {
                                return;
                            };
                            let (inbox, overdue) = {
                                let calendar = &config.peek().calendar;
                                if tab == TaskTab::Overdue {
                                    (calendar.inbox_sort, picked)
                                } else {
                                    (picked, calendar.overdue_sort)
                                }
                            };
                            match timeslay_core::config::save_task_sorts(inbox, overdue) {
                                Ok(()) => {
                                    sort_error.set(None);
                                    config.set(day::load_config());
                                }
                                Err(error) => sort_error.set(Some(format!("Could not save the order: {error}"))),
                            }
                        },
                        for choice in InboxSort::ALL {
                            option {
                                key: "{choice.name()}",
                                value: "{choice.name()}",
                                selected: choice == chosen,
                                "{choice}"
                            }
                        }
                    }
                }
            }
            if let Some(message) = sort_error() {
                p { class: "editor-error", role: "alert", "{message}" }
            }
            {invitations}
            if count == 0 {
                p { class: "empty-note", "{empty}" }
            }
            div { class: "plan-cards", {cards.into_iter()} }
        }
    }
}

/// One field of a task card.
fn field(label: &'static str, value: Element) -> Element {
    rsx! {
        div { class: "plan-field",
            span { class: "plan-field-label", "{label}" }
            {value}
        }
    }
}

fn plain(value: String) -> Element {
    rsx! { span { "{value}" } }
}

/// A missing value, called out in red while the task is still in the Inbox.
fn missing(value: &'static str, strict: bool) -> Element {
    rsx! { span { class: if strict { "plan-missing" } else { "hint" }, "{value}" } }
}

/// A task and what it carries. Tapping it opens the task's sheet, where it is
/// triaged, planned, completed or moved.
fn task_card(
    task: &Event,
    tab: TaskTab,
    config: &Config,
    time_format: TimeFormat,
    editing: Signal<Option<Event>>,
) -> Element {
    let mut editing = editing;
    let opened = task.clone();
    let m = tb::metadata(task);
    let mut fields: Vec<Element> = Vec::new();
    if tab == TaskTab::Overdue {
        fields.push(field(
            "Due Date",
            plain(scheduled_label(task, time_format).unwrap_or_else(|| "None".into())),
        ));
        if let Ok(Some(deadline)) = deadline::read_deadline(&task.details.description) {
            fields.push(field("Deadline", plain(deadline.value())));
        }
        fields.push(field(
            "Estimate",
            plain(if m.minutes == 0 {
                "No estimate".into()
            } else {
                duration(m.minutes)
            }),
        ));
    } else {
        let strict = tab == TaskTab::Inbox;
        fields.push(field(
            "Created",
            rsx! { span { class: "hint", "{timestamp(m.created_at, time_format)}" } },
        ));
        fields.push(field(
            "Modified",
            rsx! { span { class: "hint", "{timestamp(task.details.task_updated_at, time_format)}" } },
        ));
        let projects: Vec<_> = m
            .projects
            .iter()
            .filter_map(|id| config.projects.get(*id))
            .collect();
        fields.push(field(
            "Project",
            if projects.is_empty() {
                missing("No project", strict)
            } else {
                let chips = projects.into_iter().map(|project| {
                    let color = project
                        .color()
                        .map(css_color)
                        .unwrap_or_else(|| "rgb(154,154,158)".to_owned());
                    rsx! {
                        span { key: "{project.id}", class: "plan-project",
                            span { class: "day-dot", style: "background: {color};" }
                            "{project.name}"
                        }
                    }
                });
                rsx! { span { class: "plan-projects", {chips} } }
            },
        ));
        fields.push(field(
            "Priority",
            match m.priority {
                Some(priority) => plain(priority.label().to_owned()),
                None if strict => missing("No priority", true),
                None => plain("None".to_owned()),
            },
        ));
        fields.push(field(
            "Estimate",
            if m.minutes == 0 {
                if strict {
                    missing("No estimate", true)
                } else {
                    plain("None".to_owned())
                }
            } else {
                plain(duration(m.minutes))
            },
        ));
        if !m.labels.is_empty() {
            let labels = m
                .labels
                .iter()
                .map(|label| format!("#{label}"))
                .collect::<Vec<_>>()
                .join(" ");
            fields.push(field("Labels", rsx! { span { class: "hint", "{labels}" } }));
        }
        if let Some(place) = m.location.and_then(|id| config.travel.place(id)) {
            fields.push(field("Location", plain(format!("📍 {}", place.name))));
        }
        if let Ok(Some(deadline)) = deadline::read_deadline(&task.details.description) {
            let class = if tb::overdue(task) {
                "plan-missing"
            } else {
                ""
            };
            fields.push(field(
                "Deadline",
                rsx! { span { class: "{class}", "{deadline.value()}" } },
            ));
        }
        if let Some(scheduled) = scheduled_label(task, time_format) {
            fields.push(field("Scheduled", plain(scheduled)));
        }
    }

    rsx! {
        div {
            key: "{task.id}",
            class: "plan-card",
            role: "button",
            tabindex: "0",
            onclick: move |_| editing.set(Some(opened.clone())),
            div { class: "plan-card-title", "{task.title}" }
            {fields.into_iter()}
        }
    }
}

/// Invitations waiting for an answer, at the top of the Inbox. Opening one
/// shows its sheet, where it is answered.
#[component]
fn Invitations(
    calendars: Vec<Calendar>,
    settings: CalendarSettings,
    revision: Signal<u32>,
    editing: Signal<Option<Event>>,
) -> Element {
    let mut editing = editing;
    // On a calendar shared with the user, "self" is that calendar's owner, so
    // only the primary calendar's invitations are the user's to answer.
    let primary = calendars
        .iter()
        .find(|calendar| calendar.primary && !calendar.is_task_list())
        .cloned();
    let found = use_resource(move || {
        let _ = revision();
        let primary = primary.clone();
        let settings = settings.clone();
        async move {
            let Some(primary) = primary else {
                return Ok(Vec::new());
            };
            worker(move || {
                calendar::sync_invitations(
                    &calendar::GoogleClient::from_settings(&settings),
                    &primary,
                )
            })
            .await
        }
    });
    let invitations = match &*found.read() {
        Some(Ok(invitations)) => invitations.clone(),
        Some(Err(message)) => {
            return rsx! { p { class: "editor-error", role: "alert", "Could not load invitations: {message}" } };
        }
        None => Vec::new(),
    };
    if invitations.is_empty() {
        return rsx! {};
    }
    rsx! {
        h3 { class: "plan-section-title", "Invitations" }
        div { class: "plan-cards",
            for invitation in invitations {
                {
                    let mut when = calendar::event_time_label(&invitation);
                    if !invitation.details.recurrence.is_empty() {
                        when = format!("Repeats · from {when}");
                    }
                    let organizer = invitation.details.organizer.clone();
                    let opened = invitation.clone();
                    rsx! {
                        div {
                            key: "{invitation.id}",
                            class: "plan-card",
                            role: "button",
                            tabindex: "0",
                            onclick: move |_| editing.set(Some(opened.clone())),
                            div { class: "plan-card-title", "{invitation.title}" }
                            div { class: "hint", "{when}" }
                            if !organizer.is_empty() {
                                div { class: "hint", "From {organizer}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The backlog, in its planning horizons. On the desktop a task is dragged
/// between them; here each row has a picker.
#[component]
fn Backlog(
    config: Signal<Config>,
    events: Vec<Event>,
    calendars: Vec<Calendar>,
    context: Option<EditContext>,
    editing: Signal<Option<Event>>,
    on_saved: EventHandler<()>,
) -> Element {
    let mut query = use_signal(String::new);
    let busy = use_signal(|| None::<String>);
    let error = use_signal(|| None::<String>);
    let settings = config.read().calendar.clone();
    let list_name = backlog_list_name(&settings, &calendars);
    let tasks = backlog_tasks(&events, &settings, &calendars);
    let wanted = query().trim().to_lowercase();
    let shown: Vec<&Event> = tasks
        .iter()
        .copied()
        .filter(|task| wanted.is_empty() || task.title.to_lowercase().contains(&wanted))
        .collect();
    let count = shown.len();
    let sections = tb::BacklogSection::ALL.into_iter().map(|section| {
        let rows: Vec<Element> = shown
            .iter()
            .filter(|task| tb::metadata(task).backlog_section == section)
            .map(|task| backlog_row(task, context.clone(), busy, error, editing, on_saved))
            .collect();
        rsx! {
            section { key: "{section.label()}", class: "plan-section",
                h3 { class: "plan-section-title", "{section.label()}" }
                div { class: "plan-cards", {rows.into_iter()} }
            }
        }
    });
    rsx! {
        div { class: "plan-list", id: "plan-list",
            div { class: "plan-head",
                div { class: "plan-titles",
                    h2 { class: "plan-title",
                        "{list_name}"
                        span { class: "count-pill", "{count}" }
                    }
                    p { class: "hint plan-subtitle", "Google Tasks parked on this list" }
                }
            }
            input {
                class: "client-input",
                r#type: "search",
                placeholder: "Search tasks…",
                aria_label: "Search tasks",
                value: "{query}",
                oninput: move |event| query.set(event.value()),
            }
            if let Some(message) = error() {
                p { class: "editor-error", role: "alert", "{message}" }
            }
            if tasks.is_empty() {
                p { class: "empty-note", "No tasks on the {list_name} list" }
            } else if shown.is_empty() {
                p { class: "empty-note", "No matching tasks" }
            }
            {sections}
        }
    }
}

/// A backlog task: its check, title, estimate and horizon.
fn backlog_row(
    task: &Event,
    context: Option<EditContext>,
    busy: Signal<Option<String>>,
    error: Signal<Option<String>>,
    editing: Signal<Option<Event>>,
    on_saved: EventHandler<()>,
) -> Element {
    let mut editing = editing;
    let mut busy = busy;
    let mut error = error;
    let m = tb::metadata(task);
    let saving = busy().as_deref() == Some(task.id.as_str());
    let opened = task.clone();
    let checked = task.clone();
    let moved = task.clone();
    let estimate = if m.minutes == 0 {
        "No estimate".to_owned()
    } else {
        duration(m.minutes)
    };
    let labels = (!m.labels.is_empty()).then(|| {
        m.labels
            .iter()
            .map(|label| format!("#{label}"))
            .collect::<Vec<_>>()
            .join(" ")
    });
    let current = m.backlog_section;
    rsx! {
        div { key: "{task.id}", class: "plan-card backlog-row",
            div { class: "backlog-line",
                button {
                    class: "backlog-check",
                    title: "Complete task",
                    aria_label: "Complete task",
                    disabled: busy().is_some(),
                    onclick: move |_| {
                        let task = checked.clone();
                        busy.set(Some(task.id.clone()));
                        error.set(None);
                        spawn(async move {
                            let result = worker(move || {
                                calendar::set_linked_task_status(&task, tb::TaskStatus::Completed)
                            })
                            .await;
                            busy.set(None);
                            match result {
                                Ok(_) => on_saved.call(()),
                                Err(message) => error.set(Some(message)),
                            }
                        });
                    },
                    "○"
                }
                button {
                    class: "backlog-title",
                    onclick: move |_| editing.set(Some(opened.clone())),
                    "{task.title}"
                    if saving {
                        span { class: "hint", " · Saving…" }
                    }
                }
            }
            div { class: "backlog-line",
                span { class: "hint", "Estimate: {estimate}" }
                div { class: "spacer" }
                select {
                    class: "client-input backlog-section",
                    aria_label: "Plan for",
                    disabled: busy().is_some() || context.is_none(),
                    onchange: move |event: FormEvent| {
                        let Some(section) = tb::BacklogSection::ALL
                            .into_iter()
                            .find(|section| section.label() == event.value())
                        else {
                            return;
                        };
                        let Some(context) = context.clone() else {
                            return;
                        };
                        if section == current {
                            return;
                        }
                        let task = moved.clone();
                        busy.set(Some(task.id.clone()));
                        error.set(None);
                        spawn(async move {
                            let result = worker(move || {
                                day::save_task(task, Change::BacklogSection(section), &context)
                            })
                            .await;
                            busy.set(None);
                            match result {
                                Ok(_) => on_saved.call(()),
                                Err(message) => error.set(Some(message)),
                            }
                        });
                    },
                    for section in tb::BacklogSection::ALL {
                        option {
                            key: "{section.label()}",
                            value: "{section.label()}",
                            selected: section == current,
                            "{section.label()}"
                        }
                    }
                }
            }
            if let Some(labels) = labels {
                div { class: "hint", "{labels}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use timeslay_core::calendar::Details;

    fn at(day: NaiveDate) -> DateTime<Local> {
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .unwrap()
    }

    fn task(id: &str, list: &str, title: &str, day: NaiveDate) -> Event {
        Event {
            id: format!("task:{id}"),
            calendar_id: list.to_owned(),
            title: title.to_owned(),
            start: at(day),
            end: at(day + Duration::days(1)),
            all_day: true,
            color: calendar::colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        }
    }

    fn calendars() -> Vec<Calendar> {
        ["Inbox", "Backlog", "Work"]
            .into_iter()
            .map(|name| Calendar {
                id: format!("tasks:{}", name.to_lowercase()),
                name: name.to_owned(),
                color: calendar::colors::DEFAULT_EVENT,
                primary: false,
                selected_by_google: true,
            })
            .collect()
    }

    #[test]
    fn each_list_takes_the_same_tasks_as_the_desktop() {
        let now = timeslay_core::clock::now();
        let today = now.date_naive();
        let events = vec![
            task("waiting", "tasks:inbox", "Waiting", tb::UNDATED),
            task("planned", "tasks:inbox", "Planned", today),
            task("late", "tasks:work", "Late", today - Duration::days(3)),
            task("parked", "tasks:backlog", "Parked", tb::UNDATED),
            Event {
                completed: true,
                ..task("done", "tasks:backlog", "Done", tb::UNDATED)
            },
        ];
        let settings = CalendarSettings::default();
        let calendars = calendars();
        let titles = |tasks: Vec<&Event>| {
            tasks
                .iter()
                .map(|task| task.title.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            titles(inbox_tasks(&events, &settings, &calendars)),
            ["Waiting"]
        );
        assert_eq!(titles(today_tasks(&events, &settings, now)), ["Planned"]);
        assert_eq!(titles(overdue_tasks(&events, &settings, now)), ["Late"]);
        assert_eq!(
            titles(backlog_tasks(&events, &settings, &calendars)),
            ["Parked"]
        );
        assert_eq!(backlog_list_name(&settings, &calendars), "Backlog");
    }

    #[test]
    fn the_backlog_puts_the_soonest_due_first() {
        let today = timeslay_core::clock::now().date_naive();
        let events = vec![
            task("someday", "tasks:backlog", "Someday", tb::UNDATED),
            task("later", "tasks:backlog", "Later", today + Duration::days(9)),
            task("soon", "tasks:backlog", "Soon", today + Duration::days(2)),
        ];
        let tasks = backlog_tasks(&events, &CalendarSettings::default(), &calendars());
        let titles: Vec<_> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["Soon", "Later", "Someday"]);
    }

    #[test]
    fn the_plan_button_names_its_period() {
        assert_eq!(PlanView::Day.label(), "Plan Day");
        assert_eq!(PlanView::TwoWeeks.label(), "Plan 2 Weeks");
        assert_eq!(PlanView::Day.range(), None);
        assert_eq!(PlanView::Month.range(), Some(Range::Month));
    }
}
