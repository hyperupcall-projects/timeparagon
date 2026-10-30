//! The browser native-messaging host (`timeslay --webextension`) and the
//! inbox request the Slack app shares with it.
//!
//! The browser starts this process for one message and expects one answer on
//! stdout. The message is a length-prefixed JSON object (Chrome and Firefox
//! both use a four-byte native-endian length). Nothing else may be written to
//! stdout: logs go to stderr.
//!
//! The host prepares the task (a Gmail link is read for its subject, exactly
//! as a pasted link is in the app), then hands it to a running TimeSlay
//! process over [`crate::ipc`]. If no panel or dock is listening it
//! writes to Google Tasks itself, so the extension still works when the app is
//! closed. The same [`Prepared`] request is what the Slack app sends.

use std::backtrace::Backtrace;
use std::io::{self, Read, Write};
use std::time::Duration;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::calendar::Event;
use crate::calendar::deadline::{self, Deadline};
use crate::config::Config;
use crate::integrations::{self, Auth};
use crate::theme::Catalog;
use crate::{inbox, ipc};

/// The name the extension and the native host manifest agree on.
pub const HOST_NAME: &str = "com.timeslay.webextension";

/// The command that adds a task.
pub const COMMAND: &str = "add-to-inbox";

/// The command that answers with the colors of the app's chosen theme, so the
/// extension's popup can wear the same palette.
pub const COMMAND_THEME: &str = "theme";

/// A native message larger than this is treated as a mistake, not read into
/// memory. Chrome's own limit is far higher; this is only a guard.
const MAX_NATIVE_MESSAGE: usize = 8 * 1024 * 1024;

/// Adding a task waits on Google's API, so it gets longer than a visibility
/// check. The browser's own timeout is longer still.
const IPC_TIMEOUT: Duration = Duration::from_secs(60);

/// Google Tasks allows 8192 characters in a task's notes. Leave a little room
/// for the truncation marker and the link.
const MAX_NOTES: usize = 8_000;

/// What the extension or Slack app asks for.
#[derive(Debug, Default, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    /// Set to false to skip reading a recognized link (Gmail) for its title.
    #[serde(default)]
    pub fetch: Option<bool>,
    /// The day the task is due, as `YYYY-MM-DD`. Google Tasks stores this.
    #[serde(default)]
    pub due: Option<String>,
    /// When the task must be finished, as `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`.
    /// Google Tasks has no such field, so it is kept in the notes.
    #[serde(default)]
    pub deadline: Option<String>,
}

/// A task ready to be written. This is also the IPC request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prepared {
    pub command: String,
    pub title: String,
    pub notes: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
}

/// The answer the browser shows. Every failure carries the full trace so the
/// popup can print it instead of a vague "something went wrong".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Set by the `theme` command: the colors the popup should use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeColors>,
}

/// The colors of the app's chosen theme, flattened to CSS-ready hex strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeColors {
    /// The theme's name, so the popup can say which one it borrowed.
    pub name: String,
    pub is_dark: bool,
    pub background: String,
    pub surface: String,
    pub surface_strong: String,
    pub text: String,
    pub text_muted: String,
    pub primary: String,
    pub primary_strong: String,
    pub on_primary: String,
    pub success: String,
    pub warning: String,
    pub danger: String,
}

/// The task Google created, as the popup can link to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub title: String,
    pub link: String,
}

impl Reply {
    fn success(event: &Event, via: &str) -> Self {
        Self {
            ok: true,
            task: Some(TaskInfo {
                id: event.id.clone(),
                title: event.title.clone(),
                link: event.details.link.clone(),
            }),
            error: None,
            trace: None,
            via: Some(via.to_owned()),
            theme: None,
        }
    }

    fn failure(error: impl Into<String>, trace: String) -> Self {
        Self {
            ok: false,
            task: None,
            error: Some(error.into()),
            trace: Some(trace),
            via: None,
            theme: None,
        }
    }

    fn colors(theme: ThemeColors) -> Self {
        Self {
            ok: true,
            task: None,
            error: None,
            trace: None,
            via: None,
            theme: Some(theme),
        }
    }
}

/// The colors of the theme the app is actually using, so the popup looks like
/// the app. Read from the same config and catalog the app reads; a user theme
/// the app can load works here too.
pub fn theme_colors() -> ThemeColors {
    let config = Config::load();
    let catalog = Catalog::load();
    theme_colors_for(&catalog, &config.appearance.theme)
}

/// The colors of one named theme in a catalog. Split out so it can be tested
/// without reading the user's config or themes directory.
fn theme_colors_for(catalog: &Catalog, name: &str) -> ThemeColors {
    let theme = &catalog.resolve(name).theme;
    let palette = theme.extended_palette();

    ThemeColors {
        name: theme.to_string(),
        is_dark: palette.is_dark,
        background: hex(palette.background.base.color),
        surface: hex(palette.background.weak.color),
        surface_strong: hex(palette.background.strong.color),
        text: hex(palette.background.base.text),
        text_muted: muted(palette.background.base.text, palette.background.base.color),
        primary: hex(palette.primary.base.color),
        primary_strong: hex(palette.primary.strong.color),
        on_primary: hex(palette.primary.base.text),
        success: hex(palette.success.base.color),
        warning: hex(palette.warning.base.color),
        danger: hex(palette.danger.base.color),
    }
}

/// One iced color as `#rrggbb`.
fn hex(color: iced::Color) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

/// A softened text color for labels, partway to the background. iced's own
/// "weak" background carries full-strength text, which is too loud for the
/// popup's secondary lines.
fn muted(text: iced::Color, background: iced::Color) -> String {
    const TOWARD_BACKGROUND: f32 = 0.45;
    let channel = |text: f32, background: f32| {
        let value = text * (1.0 - TOWARD_BACKGROUND) + background * TOWARD_BACKGROUND;
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(text.r, background.r),
        channel(text.g, background.g),
        channel(text.b, background.b)
    )
}

/// The steps a request went through, kept so a failure can say where it broke.
#[derive(Debug, Default)]
struct Trace {
    steps: Vec<String>,
}

impl Trace {
    fn step(&mut self, step: impl Into<String>) {
        self.steps.push(step.into());
    }

    fn finish(&self, error: &str) -> String {
        let steps = if self.steps.is_empty() {
            "(none)".to_owned()
        } else {
            self.steps.join("\n")
        };
        let backtrace = Backtrace::force_capture();
        format!("{error}\n\nSteps:\n{steps}\n\nBacktrace:\n{backtrace}")
    }
}

/// A reply and the event it created, when the caller is the UI and needs to
/// fold that event into its calendar.
pub struct Handled {
    pub reply: String,
    pub event: Option<Event>,
}

/// Runs the native-messaging host for one message.
pub fn run() -> anyhow::Result<()> {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("timeslay=info"),
    )
    .try_init();

    let mut stdin = io::stdin().lock();
    let message = match read_message(&mut stdin) {
        Ok(Some(message)) => message,
        Ok(None) => return Ok(()),
        Err(error) => {
            let trace =
                Trace::default().finish(&format!("could not read the native message: {error}"));
            write_reply(&Reply::failure(
                format!("Could not read the browser's message: {error}"),
                trace,
            ))?;
            return Ok(());
        }
    };

    let reply = match serde_json::from_value::<Request>(message) {
        Ok(request) => handle_request(request),
        Err(error) => {
            let trace = Trace::default().finish(&format!("invalid request: {error}"));
            Reply::failure(
                format!("The browser sent an invalid request: {error}"),
                trace,
            )
        }
    };
    write_reply(&reply)?;
    Ok(())
}

fn write_reply(reply: &Reply) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    write_message(&mut stdout, reply)?;
    Ok(())
}

/// Handles one request from the browser. Pure except for the network and IPC
/// calls; [`prepare`] is tested on its own.
pub fn handle_request(request: Request) -> Reply {
    match request.command.as_str() {
        "" | COMMAND => {}
        COMMAND_THEME => return Reply::colors(theme_colors()),
        other => {
            let trace = Trace::default().finish(&format!("unknown command {other:?}"));
            return Reply::failure(
                format!(
                    "Unknown command {other:?}; this host understands {COMMAND} and {COMMAND_THEME}"
                ),
                trace,
            );
        }
    }

    let settings = Config::load().calendar;
    let mut trace = Trace::default();
    trace.step(format!(
        "request: url={:?}, title={:?}, source={:?}, due={:?}, deadline={:?}",
        request.url, request.title, request.source, request.due, request.deadline
    ));
    let prepared = match prepare(&request, &settings) {
        Ok(prepared) => prepared,
        Err(error) => return Reply::failure(error.clone(), trace.finish(&error)),
    };
    trace.step(format!("prepared title: {:?}", prepared.title));

    let payload = match serde_json::to_string(&prepared) {
        Ok(payload) => payload,
        Err(error) => {
            return Reply::failure(
                format!("Could not encode the request: {error}"),
                trace.finish(&error.to_string()),
            );
        }
    };

    // A running panel or dock owns the Google session and the local
    // task database, so ask it first. Only one can be listening on each socket.
    for socket in [ipc::SIDEBAR, ipc::DOCK] {
        trace.step(format!("asking {socket}"));
        match ipc::request_with_timeout(socket, &payload, IPC_TIMEOUT) {
            Ok(reply) if reply.starts_with('{') => match serde_json::from_str::<Reply>(&reply) {
                Ok(mut reply) => {
                    reply.via = Some(socket.to_owned());
                    return reply;
                }
                Err(error) => trace.step(format!("{socket} answered unreadable JSON: {error}")),
            },
            Ok(reply) if reply.is_empty() => {
                trace.step(format!("{socket} closed without an answer"))
            }
            Ok(reply) => trace.step(format!("{socket} answered {reply:?}")),
            Err(error) => trace.step(format!("{socket}: {error:#}")),
        }
    }

    trace.step("no running TimeSlay answered; writing to Google Tasks directly");
    match write_task(&settings, &prepared) {
        Ok(event) => Reply::success(&event, "direct"),
        Err(error) => Reply::failure(error.clone(), trace.finish(&error)),
    }
}

/// Handles a JSON inbox request on a TimeSlay IPC socket. The caller is a
/// running UI process, so the returned event can be added to its calendar.
pub fn handle_ipc(command: &str) -> Handled {
    let mut trace = Trace::default();
    let prepared = match serde_json::from_str::<Prepared>(command) {
        Ok(prepared) => prepared,
        Err(error) => {
            let error = format!("The inbox request is not valid JSON: {error}");
            return Handled {
                reply: failure_json(&error, trace.finish(&error)),
                event: None,
            };
        }
    };
    if !prepared.command.is_empty() && prepared.command != COMMAND {
        let error = format!("Unknown command {:?}; expected {COMMAND}", prepared.command);
        return Handled {
            reply: failure_json(&error, trace.finish(&error)),
            event: None,
        };
    }
    if prepared.title.trim().is_empty() {
        let error = "The inbox request has no title".to_owned();
        return Handled {
            reply: failure_json(&error, trace.finish(&error)),
            event: None,
        };
    }

    trace.step(format!("writing {:?} to the inbox", prepared.title));
    let settings = Config::load().calendar;
    match write_task(&settings, &prepared) {
        Ok(event) => {
            let reply = Reply::success(&event, "ipc");
            Handled {
                reply: serde_json::to_string(&reply).unwrap_or_else(|error| {
                    failure_json(
                        &format!("Could not encode the reply: {error}"),
                        trace.finish(&error.to_string()),
                    )
                }),
                event: Some(event),
            }
        }
        Err(error) => Handled {
            reply: failure_json(&error, trace.finish(&error)),
            event: None,
        },
    }
}

/// Writes a prepared task. The due day and the deadline were already validated
/// by [`prepare`], so a failure here is Google's, not a parse error.
fn write_task(
    settings: &crate::config::CalendarSettings,
    prepared: &Prepared,
) -> Result<Event, String> {
    let due = match prepared.due.as_deref() {
        Some(text) => Some(deadline::parse_date(text)?),
        None => None,
    };
    let deadline = match prepared.deadline.as_deref() {
        Some(text) => Some(deadline::parse_input(text)?),
        None => None,
    };
    inbox::add(
        settings,
        prepared.title.trim(),
        &prepared.notes,
        due,
        deadline,
    )
}

fn failure_json(error: &str, trace: String) -> String {
    serde_json::to_string(&Reply::failure(error, trace))
        .unwrap_or_else(|_| format!(r#"{{"ok":false,"error":{error:?}}}"#))
}

/// Turns a raw request into the exact title, notes, due day and deadline to
/// write. A recognized link (Gmail) is read for its subject, just like a
/// pasted link in the app.
fn prepare(
    request: &Request,
    settings: &crate::config::CalendarSettings,
) -> Result<Prepared, String> {
    let url = request.url.trim();
    let description = request.description.trim();
    if url.is_empty() && description.is_empty() && request.title.trim().is_empty() {
        return Err("Nothing to add: the request has no link, title or text".to_owned());
    }
    if !url.is_empty() && !is_web_url(url) {
        return Err(format!("{url:?} is not an http(s) link"));
    }

    let mut title = request.title.trim().to_owned();
    let mut notes = description.to_owned();

    // Validate what the person typed. `<input type="datetime-local">` uses a
    // `T` between the day and the time; the app's own fields use a space.
    let mut due: Option<NaiveDate> = match clean(request.due.as_deref()) {
        Some(text) => Some(deadline::parse_date(&text)?),
        None => None,
    };
    let mut deadline_value: Option<Deadline> = match clean(request.deadline.as_deref()) {
        Some(text) => Some(deadline::parse_input(&text)?),
        None => None,
    };

    if request.fetch.unwrap_or(true) && !url.is_empty() {
        if let Some(source) = integrations::recognize(url) {
            let auth = Auth {
                settings: settings.clone(),
            };
            let details = integrations::fetch(&source, &auth)
                .map_err(|error| format!("Could not read {url}: {error}"))?;
            // A title the person wrote is theirs; only a blank one (or the bare
            // link) is replaced by what the page calls itself.
            if !details.title.trim().is_empty() && (title.is_empty() || title == url) {
                title = details.title;
            }
            if due.is_none() {
                due = details.due;
            }
            if deadline_value.is_none() {
                deadline_value = details.deadline;
            }
        }
    }

    if title.is_empty() {
        title = first_line(&notes)
            .or_else(|| (!url.is_empty()).then(|| url.to_owned()))
            .unwrap_or_else(|| "New task".to_owned());
    }

    if !url.is_empty() && !notes.contains(url) {
        if notes.is_empty() {
            notes = url.to_owned();
        } else {
            notes.push_str("\n\n");
            notes.push_str(url);
        }
    }
    let notes = clamp_notes(&notes, url);

    Ok(Prepared {
        command: COMMAND.to_owned(),
        title,
        notes,
        url: url.to_owned(),
        source: request.source.clone(),
        due: due.map(|day| day.to_string()),
        deadline: deadline_value.map(|deadline| deadline.value()),
    })
}

/// Trims a field and treats it as absent when it is empty. The `T` an
/// `<input type="datetime-local">` puts between the day and the time becomes
/// the space the app's date fields use.
fn clean(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.replace('T', " "))
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(200).collect())
}

fn is_web_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

/// Google Tasks rejects notes longer than 8192 characters. Keep the link and
/// as much of the message as fits, with a marker where the rest was cut.
fn clamp_notes(notes: &str, url: &str) -> String {
    if notes.chars().count() <= MAX_NOTES {
        return notes.to_owned();
    }
    let marker = "\n\n[truncated by TimeSlay: the message was longer than Google Tasks allows]";
    if !url.is_empty() {
        if let Some(at) = notes.rfind(url) {
            let before = &notes[..at];
            let after = &notes[at..];
            let room = MAX_NOTES.saturating_sub(marker.chars().count() + after.chars().count());
            let head: String = before.chars().take(room).collect();
            return format!("{head}{marker}{after}");
        }
    }
    let room = MAX_NOTES.saturating_sub(marker.chars().count());
    let head: String = notes.chars().take(room).collect();
    format!("{head}{marker}")
}

/// Reads one length-prefixed JSON message. `None` means stdin closed before a
/// length arrived.
pub fn read_message(reader: &mut impl Read) -> io::Result<Option<Value>> {
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_ne_bytes(length) as usize;
    if length == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native message has an empty body",
        ));
    }
    if length > MAX_NATIVE_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("native message is {length} bytes, over the {MAX_NATIVE_MESSAGE}-byte limit"),
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let value = serde_json::from_slice(&body).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("native message is not JSON: {error}"),
        )
    })?;
    Ok(Some(value))
}

/// Writes one length-prefixed JSON message, in the byte order the browser
/// expects.
pub fn write_message(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let body = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let length = u32::try_from(body.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "native message is too large to encode",
        )
    })?;
    writer.write_all(&length.to_ne_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn settings() -> crate::config::CalendarSettings {
        crate::config::CalendarSettings::default()
    }

    #[test]
    fn native_messages_round_trip() {
        let reply = Reply {
            ok: true,
            task: Some(TaskInfo {
                id: "task:1".into(),
                title: "Hello".into(),
                link: "https://tasks.google.com/1".into(),
            }),
            error: None,
            trace: None,
            via: Some("test".into()),
            theme: None,
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &reply).unwrap();
        let mut cursor = Cursor::new(bytes);
        let value = read_message(&mut cursor).unwrap().unwrap();
        let back: Reply = serde_json::from_value(value).unwrap();
        assert_eq!(back.ok, true);
        assert_eq!(back.task.unwrap().title, "Hello");
    }

    #[test]
    fn an_empty_input_has_no_message() {
        assert!(
            read_message(&mut Cursor::new(Vec::new()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_plain_web_page_keeps_its_title_and_link() {
        let request = Request {
            url: "https://example.org/article".into(),
            title: "An article".into(),
            description: "Read this".into(),
            ..Default::default()
        };
        let prepared = prepare(&request, &settings()).unwrap();
        assert_eq!(prepared.title, "An article");
        assert_eq!(prepared.notes, "Read this\n\nhttps://example.org/article");
        assert_eq!(prepared.command, COMMAND);
    }

    #[test]
    fn a_link_with_no_title_uses_the_link_as_the_title() {
        let request = Request {
            url: "https://example.org/".into(),
            ..Default::default()
        };
        let prepared = prepare(&request, &settings()).unwrap();
        assert_eq!(prepared.title, "https://example.org/");
        assert_eq!(prepared.notes, "https://example.org/");
    }

    #[test]
    fn a_slack_description_keeps_its_link_once() {
        let request = Request {
            url: "https://workspace.slack.com/archives/C1/p123".into(),
            title: "A message".into(),
            description: "From: @a\n\nSlack: https://workspace.slack.com/archives/C1/p123\n\nHello"
                .into(),
            source: Some("slack".into()),
            ..Default::default()
        };
        let prepared = prepare(&request, &settings()).unwrap();
        assert_eq!(
            prepared
                .notes
                .matches("https://workspace.slack.com")
                .count(),
            1
        );
    }

    #[test]
    fn a_request_with_nothing_to_add_is_rejected() {
        let request = Request::default();
        assert!(prepare(&request, &settings()).is_err());
    }

    #[test]
    fn a_non_web_link_is_rejected() {
        let request = Request {
            url: "javascript:alert(1)".into(),
            ..Default::default()
        };
        assert!(prepare(&request, &settings()).is_err());
    }

    #[test]
    fn a_long_message_is_cut_but_keeps_the_link() {
        let url = "https://example.org/a";
        let notes = format!("{}\n\n{url}", "x".repeat(MAX_NOTES + 100));
        let clamped = clamp_notes(&notes, url);
        assert!(clamped.chars().count() <= MAX_NOTES);
        assert!(clamped.contains(url));
        assert!(clamped.contains("truncated by TimeSlay"));
    }

    #[test]
    fn an_unknown_command_is_rejected_with_a_trace() {
        let reply = handle_request(Request {
            command: "delete-everything".into(),
            ..Default::default()
        });
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("Unknown command"));
        assert!(reply.trace.unwrap().contains("Backtrace"));
    }

    #[test]
    fn a_bad_ipc_request_gets_an_error_reply_with_a_trace() {
        let handled = handle_ipc("{not json");
        assert!(handled.event.is_none());
        let reply: Reply = serde_json::from_str(&handled.reply).unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("not valid JSON"));
        assert!(reply.trace.unwrap().contains("Steps"));
    }

    #[test]
    fn the_same_request_the_slack_app_sends_is_accepted() {
        let prepared = Prepared {
            command: COMMAND.into(),
            title: "A Slack message".into(),
            notes: "Hello\n\nhttps://workspace.slack.com/archives/C1/p123".into(),
            url: "https://workspace.slack.com/archives/C1/p123".into(),
            source: Some("slack".into()),
            due: Some("2026-11-02".into()),
            deadline: Some("2026-11-02 17:00".into()),
        };
        let json = serde_json::to_string(&prepared).unwrap();
        let back: Prepared = serde_json::from_str(&json).unwrap();
        assert_eq!(back.title, "A Slack message");
        assert_eq!(back.source.as_deref(), Some("slack"));
        assert_eq!(back.due.as_deref(), Some("2026-11-02"));
        assert_eq!(back.deadline.as_deref(), Some("2026-11-02 17:00"));
    }

    #[test]
    fn a_due_day_and_a_deadline_are_carried_through() {
        let request = Request {
            url: "https://example.org/".into(),
            title: "Ship it".into(),
            due: Some("2026-11-02".into()),
            // The `T` an <input type="datetime-local"> produces.
            deadline: Some("2026-11-02T17:00".into()),
            ..Default::default()
        };
        let prepared = prepare(&request, &settings()).unwrap();
        assert_eq!(prepared.due.as_deref(), Some("2026-11-02"));
        assert_eq!(prepared.deadline.as_deref(), Some("2026-11-02 17:00"));
    }

    #[test]
    fn a_deadline_without_a_time_is_accepted() {
        let request = Request {
            url: "https://example.org/".into(),
            title: "Ship it".into(),
            deadline: Some("2026-11-02".into()),
            ..Default::default()
        };
        let prepared = prepare(&request, &settings()).unwrap();
        assert_eq!(prepared.due, None);
        assert_eq!(prepared.deadline.as_deref(), Some("2026-11-02"));
    }

    #[test]
    fn a_bad_due_or_deadline_is_rejected_with_a_message() {
        for request in [
            Request {
                url: "https://example.org/".into(),
                title: "x".into(),
                due: Some("tomorrow".into()),
                ..Default::default()
            },
            Request {
                url: "https://example.org/".into(),
                title: "x".into(),
                deadline: Some("2026-13-40".into()),
                ..Default::default()
            },
        ] {
            let error = prepare(&request, &settings()).unwrap_err();
            assert!(error.contains("YYYY-MM-DD"), "{error}");
        }
    }

    #[test]
    fn the_theme_reply_carries_every_color_as_hex() {
        let colors = theme_colors_for(&Catalog::built_in(), "Sakura");
        for color in [
            &colors.background,
            &colors.surface,
            &colors.surface_strong,
            &colors.text,
            &colors.text_muted,
            &colors.primary,
            &colors.primary_strong,
            &colors.on_primary,
            &colors.success,
            &colors.warning,
            &colors.danger,
        ] {
            assert!(color.starts_with('#') && color.len() == 7, "{color}");
        }
        assert_eq!(colors.name, "Sakura");
    }

    #[test]
    fn an_unknown_theme_name_still_answers_with_colors() {
        let colors = theme_colors_for(&Catalog::built_in(), "Not A Theme");
        assert!(colors.background.starts_with('#'));
        assert!(!colors.name.is_empty());
    }
}
