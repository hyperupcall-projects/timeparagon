//! The People screen: contacts grouped under the account they belong to, and an
//! editor for the chosen one in the side panel.

use super::layout::Metrics;
use crate::app::{Message, NavigationMessage, Screen, SettingsMessage, State};
use crate::people::{Action, Book, Contact, Draft, Field, List, Next, Target};
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};

fn people(action: Action) -> Message {
    Message::People(action)
}

fn bold() -> iced::Font {
    iced::Font {
        weight: iced::font::Weight::Bold,
        ..iced::Font::DEFAULT
    }
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let store = &state.people;
    let total: usize = store
        .shown_books()
        .map(|book| {
            let directory = if store.searches_directory() {
                book.directory.len()
            } else {
                0
            };
            store.visible(book).len() + directory
        })
        .sum();
    let refresh = crate::ui::controls::quiet_button(text("Refresh").size(metrics.label()))
        .on_press_maybe(
            (!store.listing && !store.books.iter().any(|book| book.loading))
                .then(|| people(Action::Refresh)),
        );
    let mut head = column![
        crate::ui::controls::page_toolbar(
            crate::ui::controls::page_heading(
                "People",
                "Contacts from your Google accounts",
                Some(total),
                metrics,
            ),
            refresh.into(),
            metrics,
        ),
        crate::ui::controls::text_input("Search names, notes and labels…", &store.query)
            .on_input(|query| people(Action::QueryChanged(query)))
            .size(metrics.label()),
    ]
    .spacing(metrics.spacing);
    if store.books.len() > 1 {
        let mut options = vec![AccountOption(None)];
        options.extend(
            store
                .books
                .iter()
                .map(|book| AccountOption(Some(book.account().to_owned()))),
        );
        head = head.push(
            crate::ui::controls::pick_list(
                options,
                Some(AccountOption(store.account.clone())),
                |option| people(Action::AccountChosen(option.0)),
            )
            .text_size(metrics.label())
            .width(Fill),
        );
    }
    if let Some(error) = &state.accounts.error {
        head = head.push(text(error).size(metrics.label()).style(text::danger));
    }

    let mut list = column![].spacing(metrics.spacing * 2.0).width(Fill);
    if store.books.is_empty() {
        list = list.push(no_accounts(state, metrics));
    }
    let domains = directory_domains(state);
    if !store.searches_directory() && !domains.is_empty() {
        list = list.push(
            text(format!("Type to also search the {domains} directory"))
                .size(metrics.label())
                .style(text::secondary),
        );
    }
    for book in store.shown_books() {
        list = list.push(section(state, book, metrics));
        if store.searches_directory()
            && (book.searching || book.directory_problem.is_some() || !book.directory.is_empty())
        {
            list = list.push(directory_section(state, book, metrics));
        }
    }

    column![
        head,
        crate::ui::controls::scrollable(
            container(list).padding(iced::Padding::default().right(12))
        )
        .height(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

/// The domains a search would reach, as a phrase.
fn directory_domains(state: &State) -> String {
    let mut domains: Vec<&str> = state
        .people
        .shown_books()
        .filter_map(Book::directory_domain)
        .collect();
    domains.dedup();
    domains.join(" and ")
}

fn no_accounts<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    if state.people.listing {
        return text("Looking for Google accounts…")
            .size(metrics.label())
            .style(text::secondary)
            .into();
    }
    column![
        text("No Google account is signed in. Connect one in Settings to see its contacts.")
            .size(metrics.label())
            .style(text::secondary),
        crate::ui::controls::action_button(text("Open Settings").size(metrics.label())).on_press(
            Message::Navigation(NavigationMessage::ScreenSelected(Screen::Settings))
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

/// One account's contacts under its address.
fn section<'a>(state: &'a State, book: &'a Book, metrics: &Metrics) -> Element<'a, Message> {
    let store = &state.people;
    let contacts = store.visible(book);
    let mut body = column![crate::ui::controls::section_heading(
        book.account(),
        Some(contacts.len()),
        metrics.label() + 1.0,
    )]
    .spacing(metrics.spacing)
    .width(Fill);
    if let Some(problem) = &book.problem {
        body = body.push(problem_line(state, book, problem, metrics));
    }
    if book.loading && book.contacts.is_empty() {
        body = body.push(quiet("Loading contacts…", metrics));
    } else if contacts.is_empty() && book.problem.is_none() {
        body = body.push(quiet(
            if book.contacts.is_empty() {
                "No contacts in this account"
            } else {
                "No matching contacts"
            },
            metrics,
        ));
    }
    body = rows(state, body, book, &contacts, false, metrics);
    body.into()
}

/// The people from the account's directory who match the search.
fn directory_section<'a>(
    state: &'a State,
    book: &'a Book,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let title = match book.directory_domain() {
        Some(domain) => format!("{domain} directory"),
        None => format!("{} directory", book.account()),
    };
    let mut body = column![crate::ui::controls::section_heading(
        title,
        Some(book.directory.len()),
        metrics.label() + 1.0,
    )]
    .spacing(metrics.spacing)
    .width(Fill);
    if let Some(problem) = &book.directory_problem {
        body = body.push(problem_line(state, book, problem, metrics));
    } else if book.searching && book.directory.is_empty() {
        body = body.push(quiet("Searching…", metrics));
    }
    let found: Vec<&Contact> = book.directory.iter().collect();
    body = rows(state, body, book, &found, true, metrics);
    body.into()
}

fn rows<'a>(
    state: &'a State,
    mut body: iced::widget::Column<'a, Message>,
    book: &'a Book,
    contacts: &[&'a Contact],
    directory: bool,
    metrics: &Metrics,
) -> iced::widget::Column<'a, Message> {
    let limit = state.people.limit();
    for contact in contacts.iter().take(limit) {
        body = body.push(contact_row(state, book, contact, directory, metrics));
    }
    if contacts.len() > limit {
        body = body.push(
            crate::ui::controls::quiet_button(
                text(format!("Show more ({} left)", contacts.len() - limit))
                    .size(metrics.label())
                    .center()
                    .width(Fill),
            )
            .width(Fill)
            .on_press(people(Action::ShowMore)),
        );
    }
    body
}

fn contact_row<'a>(
    state: &'a State,
    book: &'a Book,
    contact: &'a Contact,
    directory: bool,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let small = (metrics.label() - 1.0).max(10.0);
    let mut lines = column![text(contact.name.as_str()).size(metrics.label())]
        .spacing(2)
        .width(Fill);
    let summary = contact.summary();
    if !summary.is_empty() {
        lines = lines.push(text(summary).size(small).style(text::secondary));
    }
    if let Some(note) = contact.note_hint(&state.people.query) {
        lines = lines.push(
            text(format!("Notes: {note}"))
                .size(small)
                .style(text::secondary),
        );
    }
    if !contact.labels.is_empty() {
        lines = lines.push(
            row(contact.labels.iter().map(|label| {
                container(text(label.as_str()).size(small))
                    .padding([0, 6])
                    .style(crate::ui::controls::tag)
                    .into()
            }))
            .spacing(4)
            .wrap(),
        );
    }
    button(lines)
        .width(Fill)
        .padding([4, 6])
        .style(crate::ui::controls::nav(
            state.people.is_open(book.account(), &contact.resource),
        ))
        .on_press(people(Action::Open(Target {
            account: book.account().to_owned(),
            resource: contact.resource.clone(),
            directory,
        })))
        .into()
}

fn quiet<'a>(message: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(message)
        .size(metrics.label())
        .style(text::secondary)
        .into()
}

/// What went wrong with an account, and the sign-in that would fix it.
fn problem_line<'a>(
    state: &'a State,
    book: &'a Book,
    problem: &'a crate::people::Problem,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut line = column![
        text(problem.message.as_str())
            .size(metrics.label())
            .style(text::danger)
    ]
    .spacing(4);
    if problem.reconnect {
        let primary = book.login.is_primary();
        let waiting = if primary {
            state.calendar.is_busy()
        } else {
            state.accounts.connecting
        };
        let message = if primary {
            Message::Settings(SettingsMessage::GoogleConnectRequested)
        } else {
            Message::Accounts(crate::integrations::accounts::Action::Add)
        };
        line = line.push(
            crate::ui::controls::action_button(
                text(if waiting {
                    "Waiting for Google…".to_owned()
                } else {
                    format!("Sign in to {} again", book.account())
                })
                .size(metrics.label()),
            )
            .on_press_maybe((!waiting).then_some(message)),
        );
        if primary && let Some(error) = &state.calendar.error {
            line = line.push(text(error).size(metrics.label()).style(text::danger));
        }
    }
    line.into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AccountOption(Option<String>);

impl std::fmt::Display for AccountOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_deref().unwrap_or("All accounts"))
    }
}

/// The open contact, in the side panel.
pub fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(draft) = &state.people.draft else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };
    let close = crate::ui::controls::quiet_button(text("Close").size(metrics.label()))
        .on_press_maybe((!draft.saving).then(|| people(Action::Close)));
    let header = column![
        row![
            text(draft.contact.name.as_str())
                .size(metrics.label() + 4.0)
                .font(bold())
                .width(Fill),
            close,
        ]
        .spacing(metrics.spacing)
        .align_y(Alignment::Center),
        text(if draft.read_only {
            format!("In the directory of {}", draft.account)
        } else {
            format!("Saved in {}", draft.account)
        })
        .size(metrics.label())
        .style(text::secondary),
    ]
    .spacing(2);

    let body: Element<'a, Message> = if draft.read_only {
        read_only(state, draft, metrics)
    } else {
        editor(state, draft, metrics)
    };
    let mut pane = column![
        header,
        crate::ui::controls::scrollable(
            container(body).padding(iced::Padding::default().right(12))
        )
        .height(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill);
    if let Some(problem) = &draft.problem {
        pane = pane.push(
            text(problem.message.as_str())
                .size(metrics.label())
                .style(text::danger),
        );
    }
    pane = pane.push(footer(draft, metrics));
    pane.into()
}

fn footer<'a>(draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    if let Some(next) = &draft.pending {
        let leaving = match next {
            Next::Close => "Close without saving your changes?",
            Next::Open(_) => "Leave this contact without saving your changes?",
        };
        return column![
            text(leaving).size(metrics.label()),
            row![
                crate::ui::controls::action_button(
                    text("Keep editing")
                        .size(metrics.label())
                        .center()
                        .width(Fill)
                )
                .width(Fill)
                .on_press(people(Action::KeepEditing)),
                button(text("Discard").size(metrics.label()).center().width(Fill))
                    .width(Fill)
                    .style(crate::ui::controls::danger)
                    .on_press(people(Action::Discard)),
            ]
            .spacing(metrics.spacing),
        ]
        .spacing(metrics.spacing)
        .into();
    }
    if draft.read_only {
        return column![].into();
    }
    let dirty = draft.dirty();
    let save = crate::ui::controls::action_button(
        text(if draft.saving {
            "Saving…"
        } else if draft.saved && !dirty {
            "Saved"
        } else {
            "Save"
        })
        .size(metrics.label())
        .center()
        .width(Fill),
    )
    .width(Fill)
    .on_press_maybe((dirty && !draft.saving).then(|| people(Action::Save)));
    let revert = crate::ui::controls::quiet_button(
        text("Undo changes")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press_maybe((dirty && !draft.saving).then(|| people(Action::Revert)));
    row![save, revert].spacing(metrics.spacing).into()
}

fn caption<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(label)
        .size(metrics.label())
        .font(bold())
        .style(text::secondary)
        .into()
}

fn line<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    field: Field,
    metrics: &Metrics,
) -> Element<'a, Message> {
    column![
        caption(label, metrics),
        crate::ui::controls::text_input(placeholder, value)
            .on_input(move |value| people(Action::FieldEdited(field, value)))
            .size(metrics.label()),
    ]
    .spacing(2)
    .width(Fill)
    .into()
}

fn editor<'a>(state: &'a State, draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    column![
        row![
            line(
                "First name",
                "First name",
                &draft.given,
                Field::GivenName,
                metrics
            ),
            line(
                "Last name",
                "Last name",
                &draft.family,
                Field::FamilyName,
                metrics
            ),
        ]
        .spacing(metrics.spacing),
        row![
            line(
                "Company",
                "Company",
                &draft.company,
                Field::Company,
                metrics
            ),
            line(
                "Job title",
                "Job title",
                &draft.title,
                Field::Title,
                metrics
            ),
        ]
        .spacing(metrics.spacing),
        entries(
            "Email",
            "Add email",
            "name@example.org",
            List::Emails,
            draft,
            metrics
        ),
        entries(
            "Phone",
            "Add phone",
            "Phone number",
            List::Phones,
            draft,
            metrics
        ),
        entries(
            "Address",
            "Add address",
            "Address",
            List::Addresses,
            draft,
            metrics
        ),
        entries(
            "Website",
            "Add website",
            "https://",
            List::Urls,
            draft,
            metrics
        ),
        labels(state, draft, metrics),
        column![
            caption("Notes", metrics),
            crate::ui::controls::text_editor(&draft.notes)
                .placeholder("Notes")
                .size(metrics.label())
                .height(Length::Fixed(120.0))
                .on_action(|action| people(Action::NotesEdited(action))),
        ]
        .spacing(2),
    ]
    .spacing(metrics.spacing * 1.5)
    .width(Fill)
    .into()
}

/// The lines of a multi-valued field, each with its type and a way to remove it.
fn entries<'a>(
    label: &'a str,
    add: &'a str,
    placeholder: &'a str,
    list: List,
    draft: &'a Draft,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut body = column![caption(label, metrics)].spacing(4).width(Fill);
    for (index, entry) in draft.entries(list).iter().enumerate() {
        let mut kinds: Vec<String> = list.kinds().iter().map(|kind| (*kind).to_owned()).collect();
        if !entry.kind.is_empty() && !kinds.contains(&entry.kind) {
            kinds.push(entry.kind.clone());
        }
        let selected = (!entry.kind.is_empty()).then(|| entry.kind.clone());
        body = body.push(
            row![
                crate::ui::controls::text_input(placeholder, &entry.value)
                    .on_input(move |value| people(Action::EntryEdited(list, index, value)))
                    .size(metrics.label())
                    .width(Fill),
                crate::ui::controls::pick_list(kinds, selected, move |kind| {
                    people(Action::EntryKindChosen(list, index, kind))
                })
                .placeholder("Type")
                .text_size(metrics.label())
                .width(Length::Fixed(88.0)),
                crate::ui::controls::quiet_button(text("×").size(metrics.label()))
                    .on_press(people(Action::EntryRemoved(list, index))),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }
    body.push(
        crate::ui::controls::quiet_button(text(add).size(metrics.label()))
            .on_press(people(Action::EntryAdded(list))),
    )
    .into()
}

/// The account's labels, on or off, and a box for making a new one.
fn labels<'a>(state: &'a State, draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    let groups = state
        .people
        .books
        .iter()
        .find(|book| book.account() == draft.account)
        .map(|book| book.groups.as_slice())
        .unwrap_or_default();
    let mut body = column![caption("Labels", metrics)].spacing(4).width(Fill);
    if !groups.is_empty() {
        body = body.push(
            row(groups.iter().map(|group| {
                button(text(group.name.as_str()).size(metrics.label()))
                    .padding([2, 8])
                    .style(crate::ui::controls::nav(
                        draft.labels.contains(&group.resource),
                    ))
                    .on_press(people(Action::LabelToggled(group.resource.clone())))
                    .into()
            }))
            .spacing(4)
            .wrap(),
        );
    }
    let submit = (!draft.new_label.trim().is_empty() && !draft.creating_label)
        .then(|| people(Action::NewLabelSubmitted));
    body.push(
        row![
            crate::ui::controls::text_input("New label", &draft.new_label)
                .on_input(|value| people(Action::FieldEdited(Field::NewLabel, value)))
                .on_submit_maybe(submit.clone())
                .size(metrics.label())
                .width(Fill),
            crate::ui::controls::quiet_button(
                text(if draft.creating_label {
                    "Adding…"
                } else {
                    "Add label"
                })
                .size(metrics.label())
            )
            .on_press_maybe(submit),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .into()
}

/// A directory entry: what the domain says about them, and a way to keep a copy
/// that can be edited.
fn read_only<'a>(state: &'a State, draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    let mut body = column![].spacing(metrics.spacing * 1.5).width(Fill);
    let contact = &draft.contact;
    for (label, value) in [("Company", &contact.company), ("Job title", &contact.title)] {
        if !value.is_empty() {
            body = body.push(
                column![caption(label, metrics), text(value).size(metrics.label())].spacing(2),
            );
        }
    }
    for (label, list) in [
        ("Email", List::Emails),
        ("Phone", List::Phones),
        ("Address", List::Addresses),
        ("Website", List::Urls),
    ] {
        let lines = draft.entries(list);
        if !lines.is_empty() {
            let mut block = column![caption(label, metrics)].spacing(2);
            for entry in lines {
                block = block.push(text(entry.value.as_str()).size(metrics.label()));
            }
            body = body.push(block);
        }
    }
    body = body.push(
        text("Your organization manages this entry, so it can't be edited here. Add it to your contacts to keep your own copy.")
            .size(metrics.label())
            .style(text::secondary),
    );
    let saved = state
        .people
        .books
        .iter()
        .find(|book| book.account() == draft.account)
        .and_then(|book| book.saved_with_email(&contact.emails));
    let action = match saved {
        Some(saved) => crate::ui::controls::action_button(
            text("Open in your contacts")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .on_press(people(Action::Open(Target {
            account: draft.account.clone(),
            resource: saved.resource.clone(),
            directory: false,
        }))),
        None => crate::ui::controls::action_button(
            text(if draft.saving {
                "Adding…"
            } else {
                "Add to your contacts"
            })
            .size(metrics.label())
            .center()
            .width(Fill),
        )
        .on_press_maybe((!draft.saving).then(|| people(Action::AddToContacts))),
    };
    body.push(action.width(Fill)).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::accounts::Login;
    use crate::people;
    use iced::Size;
    use iced_test::{Simulator, core::Settings};
    use serde_json::json;

    fn contact(resource: &str, name: &str, notes: &str, groups: &[people::Group]) -> Contact {
        Contact::parse(
            json!({
                "resourceName": resource,
                "names": [{ "displayName": name, "givenName": name }],
                "emailAddresses": [{ "value": format!("{name}@example.org"), "type": "work" }],
                "biographies": [{ "value": notes }],
                "memberships": groups.iter().map(|group| json!({
                    "contactGroupMembership": { "contactGroupResourceName": group.resource }
                })).collect::<Vec<_>>(),
            }),
            groups,
        )
        .unwrap()
    }

    fn state() -> State {
        let family = people::Group {
            resource: "contactGroups/1".to_owned(),
            name: "Family".to_owned(),
        };
        let mut state = State::blank();
        state.screen = Screen::People;
        let mut personal = people::Book::for_tests(Login::extra("me@gmail.com"));
        personal.groups = vec![family.clone()];
        personal.contacts = vec![contact("people/1", "Ada", "Likes tea", &[family])];
        let mut work = people::Book::for_tests(Login::extra("me@csumb.edu"));
        work.contacts = vec![contact("people/2", "Grace", "", &[])];
        work.directory = vec![contact("people/3", "Hedy", "", &[])];
        state.people.books = vec![personal, work];
        state
    }

    fn render<'a>(state: &'a State, width: f32) -> Simulator<'a, Message> {
        Simulator::with_size(
            Settings::default(),
            Size::new(width, 900.0),
            view(state, &Metrics::of(Size::new(width, 900.0))),
        )
    }

    #[test]
    fn every_contact_sits_under_its_account() {
        let state = state();
        for width in [200.0, 280.0, 420.0, 900.0] {
            let mut ui = render(&state, width);
            for label in ["me@gmail.com", "Ada", "Family", "me@csumb.edu", "Grace"] {
                assert!(ui.find(label).is_ok(), "{label} missing at {width}");
            }
        }
    }

    #[test]
    fn clicking_a_contact_opens_it() {
        let state = state();
        let mut ui = render(&state, 420.0);
        ui.click("Ada").unwrap();
        assert!(ui.into_messages().any(|message| matches!(
            message,
            Message::People(Action::Open(target))
                if target.resource == "people/1" && target.account == "me@gmail.com"
        )));
    }

    #[test]
    fn a_search_narrows_to_notes_and_labels_and_shows_the_directory() {
        let mut state = state();
        state.people.query = "tea".to_owned();
        {
            let mut ui = render(&state, 420.0);
            assert!(ui.find("Notes: Likes tea").is_ok());
            assert!(ui.find("No matching contacts").is_ok());
            assert!(ui.find("Grace").is_err());
            assert!(ui.find("csumb.edu directory").is_ok());
            assert!(ui.find("Hedy").is_ok());
        }
        state.people.query = "family".to_owned();
        let mut ui = render(&state, 420.0);
        assert!(ui.find("Ada").is_ok());
    }

    #[test]
    fn the_directory_is_offered_only_for_workspace_accounts() {
        let state = state();
        let mut ui = render(&state, 420.0);
        assert!(
            ui.find("Type to also search the csumb.edu directory")
                .is_ok()
        );
    }

    #[test]
    fn long_lists_are_drawn_a_page_at_a_time() {
        let mut state = state();
        state.people.books[1].contacts = (0..people::PAGE + 5)
            .map(|index| contact(&format!("people/x{index}"), &format!("P{index}"), "", &[]))
            .collect();
        let mut ui = render(&state, 420.0);
        assert!(ui.find("Show more (5 left)").is_ok());
    }

    #[test]
    fn an_account_needing_sign_in_says_so_and_offers_it() {
        let mut state = state();
        state.people.books[0].problem = Some(people::Problem::sign_in("Sign in again to allow"));
        let mut ui = render(&state, 420.0);
        assert!(ui.find("Sign in again to allow").is_ok());
        ui.click("Sign in to me@gmail.com again").unwrap();
        assert!(ui.into_messages().any(|message| matches!(
            message,
            Message::Accounts(crate::integrations::accounts::Action::Add)
        )));
    }

    #[test]
    fn with_no_accounts_the_way_forward_is_settings() {
        let mut state = State::blank();
        state.screen = Screen::People;
        let mut ui = render(&state, 280.0);
        assert!(ui.find("Open Settings").is_ok());
    }

    fn open(state: &mut State, account: &str, resource: &str, directory: bool) {
        let _ = people::update(
            state,
            Action::Open(Target {
                account: account.to_owned(),
                resource: resource.to_owned(),
                directory,
            }),
        );
    }

    #[test]
    fn the_panel_takes_the_side_window_and_edits_the_contact() {
        let mut state = state();
        state.main_window = Some(iced::window::Id::unique());
        assert!(!state.people_panel_open());
        open(&mut state, "me@gmail.com", "people/1", false);
        assert!(state.people_panel_open());
        assert!(state.floating_editors());
        let size = Size::new(360.0, 900.0);
        let mut ui =
            Simulator::with_size(Settings::default(), size, crate::ui::editor_window(&state));
        for label in [
            "Saved in me@gmail.com",
            "First name",
            "Email",
            "Add phone",
            "Labels",
            "Family",
            "Notes",
            "Save",
            "Close",
        ] {
            assert!(ui.find(label).is_ok(), "{label} missing");
        }
    }

    #[test]
    fn a_contact_replaces_the_week_in_the_side_window_until_closed() {
        let mut state = state();
        state.view = crate::app::CalendarView::Week;
        open(&mut state, "me@gmail.com", "people/1", false);
        assert!(state.floating_editors());
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(360.0, 900.0),
            crate::ui::editor_window(&state),
        );
        assert!(ui.find("Saved in me@gmail.com").is_ok());
        drop(ui);
        state.screen = Screen::Settings;
        assert!(!state.people_panel_open());
    }

    #[test]
    fn a_directory_entry_offers_a_copy_rather_than_an_editor() {
        let mut state = state();
        open(&mut state, "me@csumb.edu", "people/3", true);
        let size = Size::new(360.0, 900.0);
        let mut ui =
            Simulator::with_size(Settings::default(), size, panel(&state, &Metrics::of(size)));
        assert!(ui.find("In the directory of me@csumb.edu").is_ok());
        assert!(ui.find("Add to your contacts").is_ok());
        assert!(ui.find("First name").is_err());
        assert!(ui.find("Save").is_err());
    }

    #[test]
    fn leaving_with_changes_asks_first() {
        let mut state = state();
        open(&mut state, "me@gmail.com", "people/1", false);
        let _ = people::update(
            &mut state,
            Action::FieldEdited(Field::GivenName, "Augusta".to_owned()),
        );
        let _ = people::update(&mut state, Action::Close);
        let size = Size::new(360.0, 900.0);
        let mut ui =
            Simulator::with_size(Settings::default(), size, panel(&state, &Metrics::of(size)));
        assert!(ui.find("Close without saving your changes?").is_ok());
        assert!(ui.find("Keep editing").is_ok());
        assert!(ui.find("Discard").is_ok());
    }
}
