//! Who is invited to an event, how they join it, and how the user answers.
//!
//! The event form stages these in a [`Form`] until Save, and writes only what
//! the user changed on top of what Google holds at that moment, so a guest who
//! replied while the form was open keeps their reply.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A guest's answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Response {
    #[default]
    #[serde(rename = "needsAction")]
    NeedsAction,
    #[serde(rename = "accepted")]
    Accepted,
    #[serde(rename = "declined")]
    Declined,
    #[serde(rename = "tentative")]
    Tentative,
}

impl Response {
    pub fn parse(value: &str) -> Self {
        match value {
            "accepted" => Self::Accepted,
            "declined" => Self::Declined,
            "tentative" => Self::Tentative,
            _ => Self::NeedsAction,
        }
    }

    pub fn api_value(self) -> &'static str {
        match self {
            Self::NeedsAction => "needsAction",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Tentative => "tentative",
        }
    }

    /// The words shown beside a guest.
    pub fn label(self) -> &'static str {
        match self {
            Self::NeedsAction => "no reply",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Tentative => "tentative",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Guest {
    pub email: String,
    pub name: String,
    pub response: Response,
    /// Google shows them as optional, and they don't count as needed.
    pub optional: bool,
}

impl Guest {
    /// The name when there is one, and the address when there is not.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.email
        } else {
            &self.name
        }
    }

    fn same_address(&self, email: &str) -> bool {
        self.email.eq_ignore_ascii_case(email)
    }
}

/// What guests may do. The defaults are Google's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    pub modify: bool,
    pub invite_others: bool,
    pub see_guests: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            modify: false,
            invite_others: true,
            see_guests: true,
        }
    }
}

/// An event's guests as Google last reported them. Meeting rooms are guests to
/// Google, but they are not listed here and are never touched by a save.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Attendance {
    pub guests: Vec<Guest>,
    pub permissions: Permissions,
    /// This copy of the event is the organizer's.
    pub organizer: bool,
    /// The user's address on the guest list, when they were invited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_email: Option<String>,
}

impl Attendance {
    pub fn can_manage(&self) -> bool {
        self.organizer || self.permissions.modify
    }

    /// A guest who cannot see the list has no list to add to.
    pub fn can_invite(&self) -> bool {
        self.can_manage() || (self.permissions.invite_others && self.permissions.see_guests)
    }

    pub fn own_response(&self) -> Option<Response> {
        let email = self.own_email.as_deref()?;
        self.guests
            .iter()
            .find(|guest| guest.same_address(email))
            .map(|guest| guest.response)
    }

    /// An invitation is what the user can answer: someone else organizes it.
    pub fn can_reply(&self) -> bool {
        !self.organizer && self.own_email.is_some()
    }

    /// Records the user's own answer, returning the one it replaced.
    pub fn set_own_response(&mut self, response: Response) -> Option<Response> {
        let email = self.own_email.clone()?;
        let guest = self.guests.iter_mut().find(|g| g.same_address(&email))?;
        Some(std::mem::replace(&mut guest.response, response))
    }
}

/// How people join the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conference {
    #[default]
    None,
    Meet,
    Zoom,
    /// Another provider's link. It is kept, since it can't be made here.
    Other,
}

impl std::fmt::Display for Conference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "No video call",
            Self::Meet => "Google Meet",
            Self::Zoom => "Zoom",
            Self::Other => "Other video call",
        })
    }
}

fn host_of(link: &str) -> &str {
    let link = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link);
    link.split(['/', '?', '#']).next().unwrap_or_default()
}

fn is_zoom_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ["zoom.us", "zoomgov.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// Whether `text` holds a link to a Zoom meeting, rather than to Zoom's site.
pub fn has_zoom_link(text: &str) -> bool {
    text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '(' | ')'))
        .any(|word| {
            let host = host_of(word);
            is_zoom_host(host) && word.len() > word.find(host).unwrap_or(0) + host.len() + 1
        })
}

/// Whether Google's JSON for an event already has a Zoom meeting on it.
fn event_has_zoom(event: &Value) -> bool {
    let joined = event["conferenceData"]["entryPoints"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|point| point["uri"].as_str().is_some_and(has_zoom_link));
    joined
        || ["location", "description"]
            .iter()
            .any(|key| event[*key].as_str().is_some_and(has_zoom_link))
}

fn event_has_meet(event: &Value) -> bool {
    event["hangoutLink"].as_str().is_some_and(|l| !l.is_empty())
}

/// Where Zoom for Google Workspace can be used on an event: its edit page in
/// the browser, signed in as `account`. The add-on has no API to call.
pub fn zoom_setup_url(html_link: &str, account: Option<&str>) -> Option<String> {
    let query = html_link.split_once('?')?.1;
    let eid = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("eid="))
        .filter(|eid| !eid.is_empty())?;
    let mut url = format!("https://calendar.google.com/calendar/r/eventedit/{eid}");
    if let Some(account) = account {
        url.push_str("?authuser=");
        url.push_str(&urlencoding::encode(account));
    }
    Some(url)
}

/// Whether an address is worth sending to Google.
pub fn is_email(value: &str) -> bool {
    let Some((name, domain)) = value.split_once('@') else {
        return false;
    };
    !name.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !value.contains(char::is_whitespace)
        && !domain.contains('@')
}

/// The Meeting section of the event form, staged until Save.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub guests: Vec<Guest>,
    pub permissions: Permissions,
    pub conference: Conference,
    /// Email guests about this save.
    pub notify: bool,
    /// What is typed into the add-guest field.
    pub query: String,
    pub problem: Option<String>,
    /// Answer an invitation for the whole series, not only this occurrence.
    pub reply_to_series: bool,
    pub can_manage: bool,
    pub can_invite: bool,
    new: bool,
    saved_guests: Vec<Guest>,
    saved_permissions: Permissions,
    saved_conference: Conference,
}

impl Default for Form {
    /// The form of an event that does not exist yet, which the user organizes.
    fn default() -> Self {
        Self {
            guests: Vec::new(),
            permissions: Permissions::default(),
            conference: Conference::None,
            notify: true,
            query: String::new(),
            problem: None,
            reply_to_series: false,
            can_manage: true,
            can_invite: true,
            new: true,
            saved_guests: Vec::new(),
            saved_permissions: Permissions::default(),
            saved_conference: Conference::None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    Query(String),
    /// Add the person chosen from the suggestions.
    Add {
        email: String,
        name: String,
    },
    /// Add what was typed, which has to be an address.
    AddTyped,
    Remove(String),
    /// Mark this address as an optional guest, or as a required one.
    Optional(String, bool),
    Conference(Conference),
    Modify(bool),
    InviteOthers(bool),
    SeeGuests(bool),
    Notify(bool),
    ReplyToSeries(bool),
}

/// What a save has to do on top of writing the event's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Written {
    /// The Zoom meeting still has to be added in the browser.
    pub zoom_needed: bool,
    /// The guest list changed.
    pub guests_changed: bool,
    pub conference_changed: bool,
    /// The event has guests once this is saved.
    pub has_guests: bool,
}

impl Form {
    pub fn from_event(details: &super::Details) -> Self {
        let attendance = &details.attendance;
        let conference = conference_of(details);
        Self {
            guests: attendance.guests.clone(),
            permissions: attendance.permissions,
            conference,
            can_manage: attendance.can_manage(),
            can_invite: attendance.can_invite(),
            new: false,
            saved_guests: attendance.guests.clone(),
            saved_permissions: attendance.permissions,
            saved_conference: conference,
            ..Self::default()
        }
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Query(query) => {
                self.query = query;
                self.problem = None;
            }
            Action::Add { email, name } => self.add(&email, name),
            Action::AddTyped => {
                let email = self.query.trim().to_owned();
                if is_email(&email) {
                    self.add(&email, String::new());
                } else if !email.is_empty() {
                    self.problem = Some("Enter an email address, or pick a person".to_owned());
                }
            }
            Action::Remove(email) if self.can_manage => {
                self.guests.retain(|guest| !guest.same_address(&email));
            }
            Action::Optional(email, optional) if self.can_manage => {
                if let Some(guest) = self.guests.iter_mut().find(|g| g.same_address(&email)) {
                    guest.optional = optional;
                }
            }
            Action::Conference(conference) if self.can_manage => self.conference = conference,
            // As in Google Calendar, a guest who can change the event can also
            // do the two lesser things, so those stop being a choice.
            Action::Modify(value) if self.can_manage => {
                self.permissions.modify = value;
                if value {
                    self.permissions.invite_others = true;
                    self.permissions.see_guests = true;
                }
            }
            Action::InviteOthers(value) if self.can_manage && !self.permissions.modify => {
                self.permissions.invite_others = value;
            }
            Action::SeeGuests(value) if self.can_manage && !self.permissions.modify => {
                self.permissions.see_guests = value;
            }
            Action::Notify(value) => self.notify = value,
            Action::ReplyToSeries(value) => self.reply_to_series = value,
            _ => {}
        }
    }

    fn add(&mut self, email: &str, name: String) {
        if !self.can_invite {
            return;
        }
        if !self.guests.iter().any(|guest| guest.same_address(email)) {
            self.guests.push(Guest {
                email: email.to_owned(),
                name,
                response: Response::NeedsAction,
                optional: false,
            });
        }
        self.query.clear();
        self.problem = None;
    }

    /// Saving with an address still in the field means to invite them. Anything
    /// else typed there is an unfinished search, and is left alone.
    pub fn add_typed_address(&mut self) {
        let email = self.query.trim().to_owned();
        if is_email(&email) {
            self.add(&email, String::new());
        }
    }

    /// Whether saving should send the user to the browser to add Zoom.
    pub fn zoom_wanted(&self) -> bool {
        self.conference == Conference::Zoom && self.saved_conference != Conference::Zoom
    }

    /// Zoom was chosen and saved, but the event has no Zoom link to show for it.
    pub fn zoom_unfinished(&self, details: &super::Details) -> bool {
        !self.new
            && self.conference == Conference::Zoom
            && self.saved_conference == Conference::Zoom
            && conference_of(details) != Conference::Zoom
    }

    /// What was just written is now the starting point for the next save.
    pub fn mark_saved(&mut self) {
        self.saved_guests = self.guests.clone();
        self.saved_permissions = self.permissions;
        self.saved_conference = self.conference;
        self.new = false;
    }

    /// Records the user's reply in a form that is open on the event.
    pub fn set_guest_response(&mut self, email: &str, response: Response) {
        for list in [&mut self.guests, &mut self.saved_guests] {
            if let Some(guest) = list.iter_mut().find(|g| g.same_address(email)) {
                guest.response = response;
            }
        }
    }

    /// Adds what the user changed to `body`, a write to the event `current`.
    /// `current` is `None` when the event is being created.
    pub fn write(&self, body: &mut Value, current: Option<&Value>, request_id: &str) -> Written {
        let mut written = Written::default();
        let existing = current.and_then(|event| event["attendees"].as_array());
        let added: Vec<&Guest> = self
            .guests
            .iter()
            .filter(|guest| {
                !self
                    .saved_guests
                    .iter()
                    .any(|s| s.same_address(&guest.email))
            })
            .collect();
        let removed: Vec<&Guest> = self
            .saved_guests
            .iter()
            .filter(|saved| !self.guests.iter().any(|g| g.same_address(&saved.email)))
            .collect();
        let optional_changed = |guest: &&Guest| {
            self.saved_guests
                .iter()
                .any(|s| s.same_address(&guest.email) && s.optional != guest.optional)
        };
        let reflagged: Vec<&Guest> = self.guests.iter().filter(optional_changed).collect();
        written.guests_changed = !added.is_empty() || !removed.is_empty() || !reflagged.is_empty();
        // An array replaces what Google holds, so everyone staying is sent as
        // Google described them: replies, rooms, optional flags and all.
        let mut attendees: Vec<Value> = existing
            .into_iter()
            .flatten()
            .filter(|attendee| {
                let email = attendee["email"].as_str().unwrap_or_default();
                !removed.iter().any(|guest| guest.same_address(email))
            })
            .cloned()
            .collect();
        for attendee in &mut attendees {
            let email = attendee["email"].as_str().unwrap_or_default().to_owned();
            if let Some(guest) = reflagged.iter().find(|guest| guest.same_address(&email)) {
                attendee["optional"] = guest.optional.into();
            }
        }
        for guest in added {
            let known = attendees
                .iter()
                .any(|attendee| guest.same_address(attendee["email"].as_str().unwrap_or_default()));
            if !known {
                let mut attendee = json!({ "email": guest.email });
                if !guest.name.is_empty() {
                    attendee["displayName"] = guest.name.clone().into();
                }
                if guest.optional {
                    attendee["optional"] = true.into();
                }
                attendees.push(attendee);
            }
        }
        written.has_guests = !attendees.is_empty();
        if written.guests_changed {
            body["attendees"] = Value::Array(attendees);
        }

        if self.can_manage && (self.new || self.permissions != self.saved_permissions) {
            body["guestsCanModify"] = self.permissions.modify.into();
            body["guestsCanInviteOthers"] = self.permissions.invite_others.into();
            body["guestsCanSeeOtherGuests"] = self.permissions.see_guests.into();
        }

        if self.can_manage && self.conference != self.saved_conference {
            written.conference_changed = true;
            let empty = Value::Null;
            let event = current.unwrap_or(&empty);
            let has_conference = event["conferenceData"].is_object();
            match self.conference {
                Conference::Meet if !event_has_meet(event) => {
                    body["conferenceData"] = json!({ "createRequest": {
                        "requestId": request_id,
                        "conferenceSolutionKey": { "type": "hangoutsMeet" },
                    }});
                }
                Conference::Zoom => {
                    let in_text = body["description"].as_str().is_some_and(has_zoom_link);
                    written.zoom_needed = !in_text && !event_has_zoom(event);
                    // The other provider's link would otherwise stay beside it.
                    if written.zoom_needed && has_conference {
                        body["conferenceData"] = Value::Null;
                    }
                }
                Conference::None if has_conference => body["conferenceData"] = Value::Null,
                _ => {}
            }
        }
        written
    }
}

fn conference_of(details: &super::Details) -> Conference {
    let meeting = details.meeting.as_str();
    if host_of(meeting).eq_ignore_ascii_case("meet.google.com") {
        Conference::Meet
    } else if has_zoom_link(meeting)
        || has_zoom_link(&details.location)
        || has_zoom_link(&details.description)
    {
        Conference::Zoom
    } else if meeting.is_empty() {
        Conference::None
    } else {
        Conference::Other
    }
}

/// The query string for a write to an event: Google drops conference changes
/// unless asked to read them, and sends no email unless told to.
pub fn write_query(body: &Value, notify: bool) -> String {
    let updates = if notify { "all" } else { "none" };
    if body.get("conferenceData").is_some() {
        format!("?sendUpdates={updates}&conferenceDataVersion=1")
    } else {
        format!("?sendUpdates={updates}")
    }
}

/// A fresh id for Google to tell one conference request from a retried one.
pub fn request_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Why Google could not make the Meet link it was asked for, if it could not.
pub fn conference_failure(response: &Value) -> Option<&'static str> {
    let status = &response["conferenceData"]["createRequest"]["status"]["statusCode"];
    (status.as_str() == Some("failure"))
        .then_some("Google Calendar could not create the Google Meet link")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Details;

    fn guest(email: &str, response: Response) -> Guest {
        Guest {
            email: email.into(),
            name: String::new(),
            response,
            optional: false,
        }
    }

    fn organized(guests: Vec<Guest>, meeting: &str) -> Details {
        Details {
            meeting: meeting.into(),
            attendance: Attendance {
                guests,
                organizer: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn saving_keeps_the_replies_and_rooms_google_holds() {
        let details = organized(
            vec![
                guest("ada@example.com", Response::NeedsAction),
                guest("alan@example.com", Response::Accepted),
            ],
            "",
        );
        let mut form = Form::from_event(&details);
        form.update(Action::Remove("ALAN@example.com".into()));
        form.update(Action::Add {
            email: "grace@example.com".into(),
            name: "Grace".into(),
        });
        // Ada replied after the form opened; a room is on the list too.
        let current = json!({"attendees": [
            {"email": "ada@example.com", "responseStatus": "accepted", "comment": "Yes!"},
            {"email": "alan@example.com", "responseStatus": "accepted"},
            {"email": "room@resource.calendar.google.com", "resource": true},
        ]});
        let mut body = json!({});
        let written = form.write(&mut body, Some(&current), "id");
        assert!(written.guests_changed);
        assert_eq!(
            body["attendees"],
            json!([
                {"email": "ada@example.com", "responseStatus": "accepted", "comment": "Yes!"},
                {"email": "room@resource.calendar.google.com", "resource": true},
                {"email": "grace@example.com", "displayName": "Grace"},
            ])
        );
    }

    #[test]
    fn guests_can_be_marked_optional_and_keep_their_replies() {
        let details = organized(vec![guest("ada@example.com", Response::Accepted)], "");
        let mut form = Form::from_event(&details);
        form.update(Action::Optional("ada@example.com".into(), true));
        form.update(Action::Add {
            email: "grace@example.com".into(),
            name: String::new(),
        });
        form.update(Action::Optional("grace@example.com".into(), true));
        let current = json!({"attendees": [
            {"email": "ada@example.com", "responseStatus": "accepted"},
        ]});
        let mut body = json!({});
        assert!(form.write(&mut body, Some(&current), "id").guests_changed);
        assert_eq!(
            body["attendees"],
            json!([
                {"email": "ada@example.com", "responseStatus": "accepted", "optional": true},
                {"email": "grace@example.com", "optional": true},
            ])
        );
        form.mark_saved();
        form.update(Action::Optional("ada@example.com".into(), false));
        let mut body = json!({});
        form.write(&mut body, Some(&current), "id");
        assert_eq!(body["attendees"][0]["optional"], false);
    }

    #[test]
    fn an_unchanged_form_writes_nothing() {
        let details = organized(
            vec![guest("ada@example.com", Response::Accepted)],
            "https://meet.google.com/abc-defg-hij",
        );
        let current = json!({
            "attendees": [{"email": "ada@example.com"}],
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "conferenceData": {"conferenceId": "abc"},
        });
        let mut body = json!({});
        let written = Form::from_event(&details).write(&mut body, Some(&current), "id");
        assert_eq!(body, json!({}));
        assert!(!written.guests_changed && !written.conference_changed && written.has_guests);
    }

    #[test]
    fn a_new_event_asks_for_meet_and_sets_its_permissions() {
        let mut form = Form::default();
        form.update(Action::Conference(Conference::Meet));
        form.update(Action::SeeGuests(false));
        let mut body = json!({});
        form.write(&mut body, None, "request-1");
        assert_eq!(
            body["conferenceData"]["createRequest"]["conferenceSolutionKey"]["type"],
            "hangoutsMeet"
        );
        assert_eq!(
            body["conferenceData"]["createRequest"]["requestId"],
            "request-1"
        );
        assert_eq!(body["guestsCanSeeOtherGuests"], false);
        assert_eq!(body["guestsCanInviteOthers"], true);
        assert_eq!(body["guestsCanModify"], false);
        assert_eq!(
            write_query(&body, true),
            "?sendUpdates=all&conferenceDataVersion=1"
        );
    }

    #[test]
    fn saving_meet_twice_does_not_replace_the_link() {
        let mut form = Form::from_event(&organized(Vec::new(), ""));
        form.update(Action::Conference(Conference::Meet));
        let mut first = json!({});
        form.write(&mut first, Some(&json!({})), "id");
        assert!(first.get("conferenceData").is_some());
        form.mark_saved();
        let mut second = json!({});
        form.write(&mut second, Some(&json!({})), "id");
        assert!(second.get("conferenceData").is_none());
        // Nor does a link Google already holds when the form lags behind it.
        let mut stale = Form::from_event(&organized(Vec::new(), ""));
        stale.update(Action::Conference(Conference::Meet));
        let mut body = json!({});
        stale.write(
            &mut body,
            Some(&json!({"hangoutLink": "https://meet.google.com/x"})),
            "id",
        );
        assert!(body.get("conferenceData").is_none());
    }

    #[test]
    fn zoom_goes_to_the_browser_only_when_there_is_no_zoom_link_yet() {
        let mut form = Form::default();
        form.update(Action::Conference(Conference::Zoom));
        let mut body = json!({});
        assert!(form.write(&mut body, None, "id").zoom_needed);
        assert!(body.get("conferenceData").is_none());

        let mut body = json!({"description": "Join at https://acme.zoom.us/j/123"});
        assert!(!form.write(&mut body, None, "id").zoom_needed);

        let mut form = Form::from_event(&organized(Vec::new(), ""));
        form.update(Action::Conference(Conference::Zoom));
        let added_meanwhile = json!({"conferenceData": {"entryPoints": [
            {"entryPointType": "video", "uri": "https://acme.zoom.us/j/123?pwd=abc"},
        ]}});
        let mut body = json!({});
        let written = form.write(&mut body, Some(&added_meanwhile), "id");
        assert!(!written.zoom_needed);
        assert!(body.get("conferenceData").is_none());

        let details = organized(Vec::new(), "https://acme.zoom.us/j/123");
        let mut body = json!({});
        let form = Form::from_event(&details);
        assert_eq!(form.conference, Conference::Zoom);
        assert!(!form.write(&mut body, Some(&json!({})), "id").zoom_needed);
    }

    #[test]
    fn switching_from_meet_to_zoom_or_none_removes_the_meet_link() {
        let details = organized(Vec::new(), "https://meet.google.com/abc");
        let current = json!({
            "hangoutLink": "https://meet.google.com/abc",
            "conferenceData": {"conferenceId": "abc"},
        });
        for choice in [Conference::Zoom, Conference::None] {
            let mut form = Form::from_event(&details);
            form.update(Action::Conference(choice));
            let mut body = json!({});
            let written = form.write(&mut body, Some(&current), "id");
            assert!(body["conferenceData"].is_null() && body.get("conferenceData").is_some());
            assert_eq!(written.zoom_needed, choice == Conference::Zoom);
            assert!(write_query(&body, false).ends_with("conferenceDataVersion=1"));
        }
    }

    #[test]
    fn a_guest_who_may_modify_can_also_invite_and_see_the_list() {
        let mut form = Form::default();
        form.update(Action::SeeGuests(false));
        form.update(Action::Modify(true));
        assert!(form.permissions.invite_others && form.permissions.see_guests);
        form.update(Action::SeeGuests(false));
        assert!(form.permissions.see_guests);
    }

    #[test]
    fn a_plain_guest_changes_nothing_but_can_reply() {
        let details = Details {
            meeting: "https://meet.google.com/abc".into(),
            attendance: Attendance {
                guests: vec![guest("me@example.com", Response::NeedsAction)],
                permissions: Permissions {
                    invite_others: false,
                    ..Default::default()
                },
                own_email: Some("me@example.com".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(details.attendance.can_reply());
        let mut form = Form::from_event(&details);
        form.update(Action::Remove("me@example.com".into()));
        form.update(Action::Conference(Conference::None));
        form.update(Action::Modify(true));
        form.update(Action::Add {
            email: "new@example.com".into(),
            name: String::new(),
        });
        let mut body = json!({});
        form.write(&mut body, Some(&json!({})), "id");
        assert_eq!(body, json!({}));
    }

    #[test]
    fn typed_guests_have_to_be_addresses_and_are_not_added_twice() {
        let mut form = Form::default();
        form.update(Action::Query("ada".into()));
        form.update(Action::AddTyped);
        assert!(form.guests.is_empty() && form.problem.is_some());
        form.update(Action::Query(" ada@example.com ".into()));
        form.update(Action::AddTyped);
        form.update(Action::Query("ADA@example.com".into()));
        form.update(Action::AddTyped);
        assert_eq!(form.guests.len(), 1);
        assert!(form.query.is_empty() && form.problem.is_none());
    }

    #[test]
    fn an_address_left_in_the_field_is_invited_on_save_but_a_search_is_not() {
        let mut form = Form::default();
        form.update(Action::Query("ada".into()));
        form.add_typed_address();
        assert!(form.guests.is_empty());
        assert_eq!(form.query, "ada");
        form.update(Action::Query("ada@example.com".into()));
        form.add_typed_address();
        assert_eq!(form.guests[0].email, "ada@example.com");
    }

    #[test]
    fn zoom_links_are_told_from_mentions_of_zoom() {
        assert!(has_zoom_link("Join: https://acme.zoom.us/j/123?pwd=x now"));
        assert!(has_zoom_link("zoomgov.com/j/123"));
        assert!(!has_zoom_link("Let's zoom out, see https://zoom.us"));
        assert!(!has_zoom_link("https://notzoom.us/j/1"));
    }

    #[test]
    fn the_zoom_page_is_the_events_edit_page_on_the_right_account() {
        assert_eq!(
            zoom_setup_url(
                "https://www.google.com/calendar/event?eid=abc123",
                Some("me@example.com")
            )
            .as_deref(),
            Some(
                "https://calendar.google.com/calendar/r/eventedit/abc123?authuser=me%40example.com"
            )
        );
        assert_eq!(zoom_setup_url("https://example.com", None), None);
    }
}
