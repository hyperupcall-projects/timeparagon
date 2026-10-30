//! The inbox: tasks that have not been triaged yet.
//!
//! New tasks land on a Google Tasks list called Inbox. A task stays in the
//! inbox until it is triaged, which here means being given a day: scheduling
//! it, timeblocking it, or finishing it all take it out of the inbox, as does
//! moving it to another list (the closest thing Google Tasks has to a project).
use chrono::{Duration, Local, NaiveDate};
use iced::Task;
use iced::widget::{Id, operation};

use crate::app::{Message, State};
use crate::calendar::deadline::{self, Deadline};
use crate::calendar::{self, Event, api};
use crate::integrations::{self, Auth, Source};
use crate::timeblocking::{self as tb, UNDATED};
use crate::{blocking, task_db};

pub use crate::calendar::scheduled::DEFAULT_LIST;
/// Widget id of the new-task text box, so it can be focused when it opens.
pub const TITLE_FIELD: &str = "inbox-title";
const MAX_TITLE_CHARS: usize = 1024;

#[derive(Default)]
pub struct Inbox {
    /// The new-task popup, while it is open.
    pub draft: Option<Draft>,
    /// The task whose day chooser is open.
    pub scheduling: Option<String>,
    /// The day typed into the open chooser.
    pub date: String,
    /// The month calendar open in the details pane: the task it schedules, and
    /// the first day of the month on show.
    pub picker: Option<(String, NaiveDate)>,
    pub error: Option<String>,
    /// The list a task was last created on, for the time before a sync has
    /// brought that list's name back.
    created_list: Option<String>,
    lookups: u64,
}

/// A task being written in the popup.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub title: String,
    pub deadline: String,
    pub due: String,
    /// Where the task came from, once a link has been recognized.
    pub source: Option<Source>,
    /// An integration is reading the link.
    pub fetching: bool,
    pub saving: bool,
    pub error: Option<String>,
    lookup: u64,
}

#[derive(Debug, Clone)]
pub enum Action {
    /// Open the new-task popup.
    NewTask,
    Cancel,
    Title(String),
    Deadline(String),
    Due(String),
    /// Forget the recognized link without touching the title.
    ClearSource,
    /// An integration's answer to the link with this lookup number.
    Fetched(u64, Result<integrations::Details, String>),
    Create,
    Created(Result<Event, String>),
    /// Open or close the day chooser under a task.
    ChooseDay(String),
    /// The day typed into the chooser changed.
    Date(String),
    /// Plan the task for this day.
    ScheduleOn(String, NaiveDate),
    /// Plan the task for the day typed into the chooser.
    ScheduleTyped(String),
    /// The list that collects new tasks was chosen in Settings.
    List(Option<String>),
    /// Open or close the month calendar in a task's details pane.
    TogglePicker(String),
    /// Show the month this many months after (or before) the one on show.
    PickerShift(i32),
}

/// Whether `calendar_id` is the list that collects new tasks.
pub fn is_inbox_list(state: &State, calendar_id: &str) -> bool {
    if let Some(chosen) = &state.config.calendar.inbox_list {
        return chosen.trim_start_matches("tasks:") == calendar_id.trim_start_matches("tasks:");
    }
    state.inbox.created_list.as_deref() == Some(calendar_id)
        || state.calendar.calendars.iter().any(|calendar| {
            calendar.id == calendar_id
                && calendar.is_task_list()
                && calendar.name.trim().eq_ignore_ascii_case(DEFAULT_LIST)
        })
}

/// A task waiting on the inbox list with nothing to say when it will be done.
pub fn untriaged(state: &State, event: &Event) -> bool {
    event.start.date_naive() == UNDATED
        && tb::in_backlog(event)
        && tb::metadata(event).schedule.is_none()
        && is_inbox_list(state, &event.calendar_id)
}

/// Every task still to be triaged, in the order they were put in order.
pub fn tasks(state: &State) -> Vec<&Event> {
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|event| untriaged(state, event))
        .collect();
    tasks.sort_by_cached_key(|event| (tb::metadata(event).order, event.title.to_lowercase()));
    tasks
}

/// Whether `calendar_id` is the list that collects triaged tasks.
pub fn is_triaged_list(state: &State, calendar_id: &str) -> bool {
    if let Some(chosen) = &state.config.calendar.triaged_list {
        return chosen.trim_start_matches("tasks:") == calendar_id.trim_start_matches("tasks:");
    }
    let default = crate::config::CalendarSettings::DEFAULT_TRIAGED_LIST;
    calendar_id
        .trim_start_matches("tasks:")
        .eq_ignore_ascii_case(default)
        || state.calendar.calendars.iter().any(|calendar| {
            calendar.id == calendar_id
                && calendar.is_task_list()
                && calendar.name.trim().eq_ignore_ascii_case(default)
        })
}

/// Every task that has been triaged, in the order they were put in order.
pub fn triaged_tasks(state: &State) -> Vec<&Event> {
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|event| {
            event.is_task() && !event.completed && is_triaged_list(state, &event.calendar_id)
        })
        .collect();
    tasks.sort_by_cached_key(|event| (tb::metadata(event).order, event.title.to_lowercase()));
    tasks
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::NewTask => {
            state.inbox.error = None;
            state.inbox.draft.get_or_insert_with(Draft::default);
            operation::focus(Id::new(TITLE_FIELD))
        }
        Action::Cancel => {
            // A task being saved cannot be taken back, so keep its popup up
            // to show whether the save worked.
            if state.inbox.draft.as_ref().is_some_and(|draft| draft.saving) {
                return Task::none();
            }
            state.inbox.draft = None;
            // A reply that is still on its way must not land in the next popup.
            state.inbox.lookups += 1;
            Task::none()
        }
        Action::Title(value) => title_changed(state, value),
        Action::Deadline(value) => edit(state, |draft| draft.deadline = value),
        Action::Due(value) => edit(state, |draft| draft.due = value),
        Action::ClearSource => edit(state, |draft| draft.source = None),
        Action::Fetched(lookup, result) => {
            fetched(state, lookup, result);
            Task::none()
        }
        Action::Create => create(state),
        Action::Created(result) => created(state, result),
        Action::ChooseDay(id) => {
            state.inbox.error = None;
            if state.inbox.scheduling.as_deref() == Some(id.as_str()) {
                state.inbox.scheduling = None;
            } else {
                state.inbox.scheduling = Some(id);
                state.inbox.date.clear();
            }
            Task::none()
        }
        Action::Date(value) => {
            state.inbox.date = value;
            state.inbox.error = None;
            Task::none()
        }
        Action::ScheduleOn(id, day) => schedule(state, id, day),
        Action::ScheduleTyped(id) => match deadline::parse_date(state.inbox.date.trim()) {
            Ok(day) => schedule(state, id, day),
            Err(error) => {
                state.inbox.error = Some(error.replace("a deadline", "a day"));
                Task::none()
            }
        },
        Action::List(list) => {
            state.config.calendar.inbox_list = list;
            crate::app::queue_save(state)
        }
        Action::TogglePicker(id) => {
            state.inbox.error = None;
            state.inbox.picker = match state.inbox.picker {
                Some((ref open, _)) if *open == id => None,
                _ => {
                    let today =
                        calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
                    Some((id, calendar::first_of_month(today)))
                }
            };
            Task::none()
        }
        Action::PickerShift(months) => {
            if let Some((_, month)) = state.inbox.picker.as_mut() {
                let step = chrono::Months::new(months.unsigned_abs());
                let shifted = if months >= 0 {
                    month.checked_add_months(step)
                } else {
                    month.checked_sub_months(step)
                };
                if let Some(shifted) = shifted {
                    *month = shifted;
                }
            }
            Task::none()
        }
    }
}

fn edit(state: &mut State, change: impl FnOnce(&mut Draft)) -> Task<Message> {
    if let Some(draft) = state.inbox.draft.as_mut().filter(|draft| !draft.saving) {
        change(draft);
        draft.error = None;
    }
    Task::none()
}

/// A link pasted in becomes the task's source, and is read straight away. A
/// link typed in a character at a time is only recognized, because every
/// prefix of it is not worth a request; creating the task reads it then.
fn title_changed(state: &mut State, value: String) -> Task<Message> {
    let Some(draft) = state.inbox.draft.as_mut().filter(|draft| !draft.saving) else {
        return Task::none();
    };
    let pasted = value.len() >= draft.title.len() + 2;
    draft.title = value;
    draft.error = None;
    let Some(source) = integrations::recognize(&draft.title) else {
        return Task::none();
    };
    draft.source = Some(source.clone());
    if !pasted {
        return Task::none();
    }

    state.inbox.lookups += 1;
    let lookup = state.inbox.lookups;
    draft.lookup = lookup;
    draft.fetching = true;
    let auth = Auth {
        settings: state.config.calendar.clone(),
    };
    blocking::run(move || integrations::fetch(&source, &auth))
        .map(move |result| Message::Inbox(Action::Fetched(lookup, result)))
}

fn fetched(state: &mut State, lookup: u64, result: Result<integrations::Details, String>) {
    let Some(draft) = state
        .inbox
        .draft
        .as_mut()
        .filter(|draft| draft.lookup == lookup && draft.fetching)
    else {
        return;
    };
    draft.fetching = false;
    match result {
        Ok(details) => {
            // Only the box still holding the bare link is replaced: if the
            // person has started writing their own title, theirs stays.
            let still_the_link = draft.title.trim().is_empty()
                || draft
                    .source
                    .as_ref()
                    .is_some_and(|source| source.url == draft.title.trim());
            if still_the_link {
                draft.title = details.title;
            }
            if let Some(deadline) = details
                .deadline
                .filter(|_| draft.deadline.trim().is_empty())
            {
                draft.deadline = deadline.value();
            }
            if let Some(due) = details.due.filter(|_| draft.due.trim().is_empty()) {
                draft.due = due.to_string();
            }
        }
        Err(error) => draft.error = Some(error),
    }
}

/// What the popup asks for, checked and ready to send.
#[derive(Debug)]
pub(crate) struct NewTask {
    title: String,
    deadline: Option<Deadline>,
    due: Option<NaiveDate>,
    source: Option<Source>,
}

pub(crate) fn validate(draft: &Draft) -> Result<NewTask, String> {
    let title = draft.title.trim().to_owned();
    if title.is_empty() {
        return Err("A task needs a name".into());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "A task name can be at most {MAX_TITLE_CHARS} characters"
        ));
    }
    let deadline = match draft.deadline.trim() {
        "" => None,
        text => Some(deadline::parse_input(text)?),
    };
    let due = match draft.due.trim() {
        "" => None,
        text => Some(
            deadline::parse_date(text).map_err(|_| "Enter a due date as YYYY-MM-DD".to_owned())?,
        ),
    };
    // A link typed out in full is a source too, even if it was never pasted.
    let source = draft
        .source
        .clone()
        .or_else(|| integrations::recognize(&title));
    Ok(NewTask {
        title,
        deadline,
        due,
        source,
    })
}

fn create(state: &mut State) -> Task<Message> {
    let Some(draft) = state.inbox.draft.as_mut() else {
        return Task::none();
    };
    if draft.saving || draft.fetching {
        return Task::none();
    }
    let task = match validate(draft) {
        Ok(task) => task,
        Err(error) => {
            draft.error = Some(error);
            return Task::none();
        }
    };
    draft.saving = true;
    draft.error = None;
    let settings = state.config.calendar.clone();
    blocking::run(move || write(&settings, task))
        .map(|result| Message::Inbox(Action::Created(result)))
}

/// Creates the task on Google. A link whose title was never read gets one more
/// try here, so a task made quickly still arrives with a proper name.
pub(crate) fn write(
    settings: &crate::config::CalendarSettings,
    task: NewTask,
) -> Result<Event, String> {
    let token = tb::access(settings)?;
    let mut title = task.title;
    if let Some(source) = task.source.as_ref().filter(|source| source.url == title) {
        let auth = Auth {
            settings: settings.clone(),
        };
        if let Ok(details) = integrations::fetch(source, &auth) {
            title = details.title;
        }
    }
    let notes = deadline::write(
        task.source
            .as_ref()
            .map_or("", |source| source.url.as_str()),
        task.deadline,
    );
    create_event(settings, &token, &title, &notes, task.due)
}

/// Adds a task that already has its title and notes, for callers outside the
/// new-task popup: the browser extension and the Slack app. The notes keep the
/// link that lets the task show its source later.
pub fn add(
    settings: &crate::config::CalendarSettings,
    title: &str,
    notes: &str,
    due: Option<NaiveDate>,
    deadline: Option<Deadline>,
) -> Result<Event, String> {
    let token = tb::access(settings)?;
    // A deadline lives in the notes, since Google Tasks has no such field.
    // With none there is nothing to add, so the notes are left exactly as the
    // caller wrote them.
    let notes = match deadline {
        Some(deadline) => deadline::write(notes, Some(deadline)),
        None => notes.to_owned(),
    };
    create_event(settings, &token, title, &notes, due)
}

/// Writes one task to the inbox list and builds the local [`Event`] for it.
fn create_event(
    settings: &crate::config::CalendarSettings,
    token: &str,
    title: &str,
    notes: &str,
    due: Option<NaiveDate>,
) -> Result<Event, String> {
    let list = calendar::scheduled::inbox_destination(token, settings.inbox_list.as_deref())?;
    let created = api::insert_task_with(token, &list, &api::NewTask { title, notes, due })?;

    let calendar_id = format!("tasks:{list}");
    let start = calendar::at_clock(due.unwrap_or(UNDATED), 0, 0);
    let mut event = Event {
        id: format!("task:{}", created.id),
        color: settings
            .task_color(&calendar_id)
            .unwrap_or(calendar::colors::TASK),
        calendar_id,
        title: created.title,
        start,
        end: start + Duration::days(1),
        all_day: true,
        completed: false,
        details: calendar::Details {
            link: created.link,
            description: notes.to_owned(),
            ..Default::default()
        },
    };
    // The local record is what carries estimates and block assignments, so it
    // exists from the first moment.
    event.details.task_data = Some(task_db::open()?.import(&event)?);
    Ok(event)
}

fn created(state: &mut State, result: Result<Event, String>) -> Task<Message> {
    match result {
        Ok(event) => {
            state.inbox.created_list = Some(event.calendar_id.clone());
            state.inbox.draft = None;
            state.timeblocking.revision += 1;
            state.calendar.events.retain(|other| other.id != event.id);
            state.calendar.add_event(event);
            state.calendar.invalidate_other_days();
            // Brings the Inbox list itself, if this made it, into the app.
            Task::done(Message::CalendarSyncRequested)
        }
        Err(error) => {
            if let Some(draft) = state.inbox.draft.as_mut() {
                draft.saving = false;
                draft.error = Some(error);
            }
            Task::none()
        }
    }
}

fn schedule(state: &mut State, id: String, day: NaiveDate) -> Task<Message> {
    state.inbox.error = None;
    state.inbox.scheduling = None;
    state.inbox.picker = None;
    tb::update(state, tb::Action::ScheduleDay(id, day))
}

/// The days the chooser offers without typing: today, tomorrow, and the
/// coming Monday.
pub fn quick_days(today: NaiveDate) -> [(&'static str, NaiveDate); 3] {
    use chrono::Datelike;
    let ahead = 7 - i64::from(today.weekday().num_days_from_monday());
    [
        ("Today", today),
        ("Tomorrow", today + Duration::days(1)),
        ("Next Monday", today + Duration::days(ahead)),
    ]
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Calendar;

    const THREAD: &str = "https://mail.google.com/mail/u/0/#inbox/FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW";

    fn state() -> State {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        let mut snapshot = calendar::sample_snapshot();
        snapshot.events.clear();
        snapshot.calendars.push(Calendar {
            id: "tasks:inbox".into(),
            name: "Inbox".into(),
            color: calendar::colors::TASK,
            primary: false,
            selected_by_google: true,
        });
        state.calendar = calendar::Store::with_snapshot(snapshot);
        state
    }

    fn task(id: &str, list: &str, day: NaiveDate) -> Event {
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = format!("task:{id}");
        task.calendar_id = format!("tasks:{list}");
        task.title = id.into();
        task.all_day = true;
        task.completed = false;
        task.start = calendar::at_clock(day, 0, 0);
        task.end = task.start + Duration::days(1);
        task.details = Default::default();
        task
    }

    fn day(text: &str) -> NaiveDate {
        deadline::parse_date(text).unwrap()
    }

    #[test]
    fn only_undated_tasks_on_the_inbox_list_are_untriaged() {
        let mut state = state();
        let mut done = task("done", "inbox", UNDATED);
        done.completed = true;
        state.calendar.events = vec![
            task("fresh", "inbox", UNDATED),
            task("planned", "inbox", day("2026-11-02")),
            task("elsewhere", "projects", UNDATED),
            done,
        ];
        let titles: Vec<_> = tasks(&state).iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["fresh"]);
    }

    #[test]
    fn a_task_given_a_block_or_a_schedule_is_triaged() {
        let mut state = state();
        let mut blocked = task("blocked", "inbox", UNDATED);
        let mut data = crate::task_db::TaskData {
            id: 100,
            metadata: Default::default(),
            legacy_key: None,
        };
        data.metadata.block = Some("work/block".into());
        blocked.details.task_data = Some(data);
        state.calendar.events = vec![blocked];
        assert!(tasks(&state).is_empty());
    }

    #[test]
    fn triaged_tasks_are_the_ones_on_the_triaged_list() {
        let mut state = state();
        state.calendar.calendars.push(Calendar {
            id: "tasks:triaged".into(),
            name: "Triaged".into(),
            color: calendar::colors::TASK,
            primary: false,
            selected_by_google: true,
        });
        let mut ready = task("ready", "triaged", UNDATED);
        ready.title = "Research outline".into();
        state.calendar.events = vec![task("fresh", "inbox", UNDATED), ready];
        assert_eq!(
            triaged_tasks(&state)
                .iter()
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>(),
            ["Research outline"]
        );

        // A chosen list replaces the one named Triaged.
        state.config.calendar.triaged_list = Some("tasks:mine".into());
        state.calendar.events.push(task("mine", "mine", UNDATED));
        assert_eq!(triaged_tasks(&state).len(), 1);
    }

    #[test]
    fn a_chosen_list_replaces_the_one_named_inbox() {
        let mut state = state();
        state.calendar.events = vec![task("a", "inbox", UNDATED), task("b", "mine", UNDATED)];
        state.config.calendar.inbox_list = Some("tasks:mine".into());
        let titles: Vec<_> = tasks(&state).iter().map(|t| t.title.clone()).collect();
        assert_eq!(titles, ["b"]);
        state.config.calendar.inbox_list = Some("mine".into());
        assert_eq!(tasks(&state).len(), 1);
    }

    #[test]
    fn the_list_a_task_was_just_created_on_counts_before_a_sync() {
        let mut state = state();
        state.calendar.calendars.clear();
        state.calendar.events = vec![task("a", "new-list", UNDATED)];
        assert!(tasks(&state).is_empty());
        let event = task("b", "new-list", UNDATED);
        let _ = created(&mut state, Ok(event));
        assert_eq!(tasks(&state).len(), 2);
    }

    #[test]
    fn the_new_task_popup_opens_and_closes() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        assert!(state.inbox.draft.is_some());
        let _ = update(&mut state, Action::Title("Call the bank".into()));
        let _ = update(&mut state, Action::Cancel);
        assert!(state.inbox.draft.is_none());
        let _ = update(&mut state, Action::NewTask);
        assert_eq!(state.inbox.draft.as_ref().unwrap().title, "");
    }

    #[test]
    fn a_pasted_link_is_read_and_a_typed_one_waits() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        let _ = update(&mut state, Action::Title(THREAD.into()));
        let draft = state.inbox.draft.as_ref().unwrap();
        assert!(draft.fetching);
        assert_eq!(draft.source.as_ref().unwrap().url, THREAD);

        let mut state = self::state();
        let _ = update(&mut state, Action::NewTask);
        // One more character at a time is typing, not pasting.
        state.inbox.draft.as_mut().unwrap().title = THREAD[..THREAD.len() - 1].into();
        let _ = update(&mut state, Action::Title(THREAD.into()));
        let draft = state.inbox.draft.as_ref().unwrap();
        assert!(!draft.fetching);
        assert!(draft.source.is_some());
    }

    fn details(title: &str) -> integrations::Details {
        integrations::Details {
            title: title.into(),
            deadline: None,
            due: None,
        }
    }

    #[test]
    fn what_an_integration_finds_fills_only_what_was_left_empty() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        let _ = update(&mut state, Action::Title(THREAD.into()));
        let _ = update(&mut state, Action::Due("2026-11-03".into()));
        let lookup = state.inbox.lookups;
        let found = integrations::Details {
            title: "Quarterly report".into(),
            deadline: Some(deadline::parse_input("2026-11-10 17:00").unwrap()),
            due: Some(day("2026-11-04")),
        };
        let _ = update(&mut state, Action::Fetched(lookup, Ok(found)));
        let draft = state.inbox.draft.as_ref().unwrap();
        assert!(!draft.fetching);
        assert_eq!(draft.title, "Quarterly report");
        assert_eq!(draft.deadline, "2026-11-10 17:00");
        assert_eq!(draft.due, "2026-11-03");
        assert!(draft.source.is_some());
    }

    #[test]
    fn a_title_the_person_wrote_is_not_replaced_by_a_late_answer() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        let _ = update(&mut state, Action::Title(THREAD.into()));
        let lookup = state.inbox.lookups;
        let _ = update(&mut state, Action::Title("My own words".into()));
        let _ = update(
            &mut state,
            Action::Fetched(lookup, Ok(details("Subject line"))),
        );
        let draft = state.inbox.draft.as_ref().unwrap();
        assert_eq!(draft.title, "My own words");
        assert!(draft.source.is_some());
    }

    #[test]
    fn an_answer_for_an_older_link_or_a_closed_popup_is_ignored() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        let _ = update(&mut state, Action::Title(THREAD.into()));
        let stale = state.inbox.lookups;
        let _ = update(&mut state, Action::Cancel);
        let _ = update(&mut state, Action::Fetched(stale, Ok(details("Old"))));
        assert!(state.inbox.draft.is_none());
        let _ = update(&mut state, Action::NewTask);
        let _ = update(&mut state, Action::Fetched(stale, Ok(details("Old"))));
        assert_eq!(state.inbox.draft.as_ref().unwrap().title, "");
    }

    #[test]
    fn a_failed_read_keeps_the_link_and_says_why() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        let _ = update(&mut state, Action::Title(THREAD.into()));
        let lookup = state.inbox.lookups;
        let _ = update(
            &mut state,
            Action::Fetched(lookup, Err("Reconnect Google".into())),
        );
        let draft = state.inbox.draft.as_ref().unwrap();
        assert_eq!(draft.title, THREAD);
        assert_eq!(draft.error.as_deref(), Some("Reconnect Google"));
        assert!(!draft.fetching);
    }

    #[test]
    fn the_popup_checks_its_fields_before_anything_is_sent() {
        let mut draft = Draft::default();
        assert_eq!(
            validate(&draft).err().as_deref(),
            Some("A task needs a name")
        );
        draft.title = "Pay rent".into();
        assert!(validate(&draft).is_ok());
        draft.deadline = "soon".into();
        assert!(validate(&draft).unwrap_err().contains("YYYY-MM-DD"));
        draft.deadline = "2026-11-01 09:00".into();
        draft.due = "tomorrow".into();
        assert_eq!(
            validate(&draft).err().as_deref(),
            Some("Enter a due date as YYYY-MM-DD")
        );
        draft.due = "2026-10-30".into();
        let task = validate(&draft).unwrap();
        assert_eq!(task.due, Some(day("2026-10-30")));
        assert_eq!(task.deadline.unwrap().value(), "2026-11-01 09:00");
        assert!(task.source.is_none());
        draft.title = format!("  {THREAD} ");
        assert_eq!(validate(&draft).unwrap().source.unwrap().url, THREAD);
        draft.title = "x".repeat(MAX_TITLE_CHARS + 1);
        assert!(validate(&draft).unwrap_err().contains("at most"));
    }

    #[test]
    fn an_invalid_task_stays_in_the_popup_with_its_error() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        let _ = update(&mut state, Action::Create);
        let draft = state.inbox.draft.as_ref().unwrap();
        assert!(!draft.saving);
        assert_eq!(draft.error.as_deref(), Some("A task needs a name"));
    }

    #[test]
    fn a_created_task_joins_the_inbox_and_closes_the_popup() {
        let mut state = state();
        let _ = update(&mut state, Action::NewTask);
        let revision = state.timeblocking.revision;
        let _ = created(&mut state, Ok(task("new", "inbox", UNDATED)));
        assert!(state.inbox.draft.is_none());
        assert_eq!(state.timeblocking.revision, revision + 1);
        assert_eq!(tasks(&state).len(), 1);

        let _ = update(&mut state, Action::NewTask);
        let _ = created(&mut state, Err("Google: nope".into()));
        let draft = state.inbox.draft.as_ref().unwrap();
        assert_eq!(draft.error.as_deref(), Some("Google: nope"));
        assert!(!draft.saving);
    }

    #[test]
    fn the_day_chooser_opens_per_task_and_rejects_bad_days() {
        let mut state = state();
        let _ = update(&mut state, Action::ChooseDay("task:a".into()));
        assert_eq!(state.inbox.scheduling.as_deref(), Some("task:a"));
        let _ = update(&mut state, Action::ChooseDay("task:b".into()));
        assert_eq!(state.inbox.scheduling.as_deref(), Some("task:b"));
        let _ = update(&mut state, Action::ChooseDay("task:b".into()));
        assert_eq!(state.inbox.scheduling, None);

        let _ = update(&mut state, Action::ChooseDay("task:a".into()));
        let _ = update(&mut state, Action::Date("next week".into()));
        let _ = update(&mut state, Action::ScheduleTyped("task:a".into()));
        assert_eq!(state.inbox.scheduling.as_deref(), Some("task:a"));
        assert!(state.inbox.error.as_deref().unwrap().contains("YYYY-MM-DD"));
    }

    #[test]
    fn choosing_a_day_hands_the_task_to_scheduling() {
        let mut state = state();
        state.calendar.events = vec![task("a", "inbox", UNDATED)];
        let _ = update(&mut state, Action::ChooseDay("task:a".into()));
        let _ = update(
            &mut state,
            Action::ScheduleOn("task:a".into(), day("2026-11-03")),
        );
        assert_eq!(state.inbox.scheduling, None);
        // The write is in flight, so the task is held until Google answers.
        assert!(state.timeblocking.busy.contains("task:a"));
    }

    #[test]
    fn the_quick_days_are_today_tomorrow_and_the_next_monday() {
        // A Monday, a Wednesday and a Sunday.
        for (today, monday) in [
            ("2026-10-26", "2026-11-02"),
            ("2026-10-28", "2026-11-02"),
            ("2026-11-01", "2026-11-02"),
        ] {
            let days = quick_days(day(today));
            assert_eq!(days[0].1, day(today));
            assert_eq!(days[1].1, day(today) + Duration::days(1));
            assert_eq!(days[2].1, day(monday));
        }
    }

    #[test]
    fn the_month_picker_opens_on_this_month_and_steps_through_the_others() {
        let mut state = state();
        let this_month = calendar::first_of_month(calendar::day_at(
            Local::now(),
            state.config.calendar.day_start_hour(),
        ));
        let _ = update(&mut state, Action::TogglePicker("task:a".into()));
        assert_eq!(state.inbox.picker, Some(("task:a".into(), this_month)));
        let _ = update(&mut state, Action::PickerShift(1));
        let _ = update(&mut state, Action::PickerShift(1));
        let _ = update(&mut state, Action::PickerShift(-3));
        let last_month = this_month - chrono::Months::new(1);
        assert_eq!(state.inbox.picker, Some(("task:a".into(), last_month)));
        // Another task starts again from this month; the same one closes it.
        let _ = update(&mut state, Action::TogglePicker("task:b".into()));
        assert_eq!(state.inbox.picker, Some(("task:b".into(), this_month)));
        let _ = update(&mut state, Action::TogglePicker("task:b".into()));
        assert_eq!(state.inbox.picker, None);
        // Nothing to step while it is closed.
        let _ = update(&mut state, Action::PickerShift(1));
        assert_eq!(state.inbox.picker, None);
    }

    #[test]
    fn a_day_picked_in_the_month_plans_the_task_and_closes_the_picker() {
        let mut state = state();
        state.calendar.events = vec![task("a", "inbox", UNDATED)];
        let _ = update(&mut state, Action::TogglePicker("task:a".into()));
        let _ = update(
            &mut state,
            Action::ScheduleOn("task:a".into(), day("2026-11-12")),
        );
        assert_eq!(state.inbox.picker, None);
        assert!(state.timeblocking.busy.contains("task:a"));
    }

    #[test]
    fn the_list_choice_is_remembered() {
        let mut state = state();
        let _ = update(&mut state, Action::List(Some("tasks:mine".into())));
        assert_eq!(
            state.config.calendar.inbox_list.as_deref(),
            Some("tasks:mine")
        );
        let _ = update(&mut state, Action::List(None));
        assert_eq!(state.config.calendar.inbox_list, None);
    }
}
