use super::*;
use serde_json::json;

fn group(id: &str, name: &str) -> Group {
    Group {
        resource: format!("contactGroups/{id}"),
        name: name.to_owned(),
    }
}

fn person() -> Value {
    json!({
        "resourceName": "people/c1",
        "etag": "person-etag",
        "metadata": { "sources": [
            { "type": "CONTACT", "id": "1", "etag": "source-etag" },
            { "type": "PROFILE", "id": "2", "etag": "profile-etag" },
        ]},
        "names": [{
            "metadata": { "primary": true },
            "displayName": "Ada Lovelace",
            "displayNameLastFirst": "Lovelace, Ada",
            "unstructuredName": "Ada Lovelace",
            "givenName": "Ada",
            "familyName": "Lovelace",
            "phoneticGivenName": "Ay-da",
        }],
        "emailAddresses": [
            { "metadata": {}, "value": "ada@example.org", "type": "work", "formattedType": "Work" },
            { "metadata": {}, "value": "ada@home.example", "type": "home" },
        ],
        "phoneNumbers": [{ "value": "(555) 010-0000", "canonicalForm": "+15550100000", "type": "mobile" }],
        "addresses": [{
            "formattedValue": "1 Main St, Seaside",
            "streetAddress": "1 Main St",
            "city": "Seaside",
            "type": "home",
        }],
        "organizations": [{ "name": "Analytical Engines", "title": "Programmer", "current": true }, { "name": "Royal Society" }],
        "biographies": [{ "metadata": {}, "value": "Met at the\ncomputing history talk", "contentType": "TEXT_PLAIN" }],
        "memberships": [
            { "contactGroupMembership": { "contactGroupResourceName": "contactGroups/myContacts", "contactGroupId": "myContacts" } },
            { "contactGroupMembership": { "contactGroupResourceName": "contactGroups/friends1", "contactGroupId": "friends1" } },
        ],
    })
}

fn contact() -> Contact {
    Contact::parse(
        person(),
        &[group("friends1", "Book club"), group("work2", "Work")],
    )
    .unwrap()
}

fn draft() -> Draft {
    Draft::new(&Login::extra("me@example.org"), &contact(), false)
}

fn book_state() -> State {
    let mut state = State::blank();
    let mut book = Book::new(Login::extra("me@example.org"));
    book.loading = false;
    book.groups = vec![group("friends1", "Book club"), group("work2", "Work")];
    book.contacts = vec![contact()];
    book.directory = vec![
        Contact::parse(
            json!({ "resourceName": "people/d1", "names": [{"displayName": "Grace Hopper"}], "emailAddresses": [{"value": "grace@example.org"}] }),
            &[],
        )
        .unwrap(),
    ];
    book.directory_seen = book.directory.clone();
    state.people.books.push(book);
    state
}

fn target(resource: &str, directory: bool) -> Target {
    Target {
        account: "me@example.org".to_owned(),
        resource: resource.to_owned(),
        directory,
    }
}

#[test]
fn search_covers_names_notes_and_labels_in_any_case() {
    let contact = contact();
    assert_eq!(contact.labels, ["Book club"]);
    for query in [
        "ada",
        "LOVELACE",
        "history talk",
        "book club",
        "club ada",
        "",
    ] {
        assert!(contact.matches(query), "{query:?} should match");
    }
    for query in ["grace", "ada grace", "Work"] {
        assert!(!contact.matches(query), "{query:?} should not match");
    }
}

#[test]
fn a_match_in_the_notes_is_explained_and_a_match_in_the_name_is_not() {
    let contact = contact();
    assert_eq!(contact.note_hint("history"), Some("computing history talk"));
    assert_eq!(contact.note_hint("ada"), None);
    assert_eq!(contact.note_hint(""), None);
}

#[test]
fn a_contact_with_no_name_goes_by_what_it_has() {
    let email =
        json!({ "resourceName": "people/a", "emailAddresses": [{"value": "x@example.org"}] });
    assert_eq!(Contact::parse(email, &[]).unwrap().name, "x@example.org");
    let company = json!({ "resourceName": "people/b", "organizations": [{"name": "Acme"}] });
    assert_eq!(Contact::parse(company, &[]).unwrap().name, "Acme");
    let nothing = json!({ "resourceName": "people/c" });
    assert_eq!(
        Contact::parse(nothing, &[]).unwrap().name,
        "Unnamed contact"
    );
    assert_eq!(Contact::parse(json!({}), &[]), None);
}

#[test]
fn an_untouched_contact_has_nothing_to_save() {
    let mut draft = draft();
    assert!(!draft.dirty());
    // Blank lines and stray spaces are not edits.
    draft.emails.push(Entry {
        value: "  ".to_owned(),
        kind: String::new(),
        source: None,
    });
    draft.given = "Ada ".to_owned();
    assert!(!draft.dirty());
}

#[test]
fn an_update_sends_only_what_changed_with_the_contact_source() {
    let mut draft = draft();
    draft.title = "Mathematician".to_owned();
    let (fields, body) = draft.patch();
    assert_eq!(fields, ["organizations"]);
    assert_eq!(body["etag"], "person-etag");
    let sources = body["metadata"]["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["etag"], "source-etag");
    assert!(body.get("names").is_none());
    assert!(body.get("emailAddresses").is_none());
    assert_eq!(
        body["organizations"],
        json!([
            { "name": "Analytical Engines", "title": "Mathematician", "current": true },
            { "name": "Royal Society" },
        ])
    );
}

#[test]
fn a_new_name_keeps_the_parts_with_no_control_and_drops_the_computed_ones() {
    let mut draft = draft();
    draft.family = "King".to_owned();
    let (fields, body) = draft.patch();
    assert_eq!(fields, ["names"]);
    assert_eq!(
        body["names"],
        json!([{ "givenName": "Ada", "familyName": "King", "phoneticGivenName": "Ay-da" }])
    );
}

#[test]
fn lines_are_edited_added_and_removed_in_place() {
    let mut draft = draft();
    draft.emails[1].value = "ada@new.example".to_owned();
    draft.emails.remove(0);
    draft.phones.push(Entry {
        value: "555-0101".to_owned(),
        kind: "work".to_owned(),
        source: None,
    });
    draft.phones[0].kind = "home".to_owned();
    let (fields, body) = draft.patch();
    assert_eq!(fields, ["emailAddresses", "phoneNumbers"]);
    assert_eq!(
        body["emailAddresses"],
        json!([{ "value": "ada@new.example", "type": "home" }])
    );
    assert_eq!(
        body["phoneNumbers"],
        json!([
            { "value": "(555) 010-0000", "type": "home" },
            { "value": "555-0101", "type": "work" },
        ])
    );
}

#[test]
fn removing_the_last_line_clears_the_field() {
    let mut draft = draft();
    draft.phones.clear();
    let (fields, body) = draft.patch();
    assert_eq!(fields, ["phoneNumbers"]);
    assert_eq!(body["phoneNumbers"], json!([]));
}

#[test]
fn a_rewritten_address_forgets_its_old_parts_and_a_relabeled_one_keeps_them() {
    let mut draft = draft();
    draft.addresses[0].kind = "work".to_owned();
    let (_, body) = draft.patch();
    assert_eq!(body["addresses"][0]["city"], "Seaside");
    assert_eq!(body["addresses"][0]["type"], "work");

    draft.addresses[0].value = "2 Ocean Ave, Marina".to_owned();
    let (_, body) = draft.patch();
    assert_eq!(
        body["addresses"],
        json!([{ "formattedValue": "2 Ocean Ave, Marina", "type": "work" }])
    );
}

#[test]
fn notes_are_replaced_and_cleared() {
    let mut draft = draft();
    draft.notes = text_editor::Content::with_text("New note");
    let (fields, body) = draft.patch();
    assert_eq!(fields, ["biographies"]);
    assert_eq!(
        body["biographies"],
        json!([{ "value": "New note", "contentType": "TEXT_PLAIN" }])
    );
    draft.notes = text_editor::Content::new();
    assert_eq!(draft.patch().1["biographies"], json!([]));
}

#[test]
fn notes_can_be_added_to_a_contact_without_any() {
    let contact = Contact::parse(json!({ "resourceName": "people/a" }), &[]).unwrap();
    let mut draft = Draft::new(&Login::extra("me@example.org"), &contact, false);
    draft.notes = text_editor::Content::with_text("First note");
    assert_eq!(
        draft.patch().1["biographies"],
        json!([{ "value": "First note", "contentType": "TEXT_PLAIN" }])
    );
}

#[test]
fn changing_labels_leaves_system_groups_alone() {
    let mut draft = draft();
    draft
        .labels
        .retain(|label| label != "contactGroups/friends1");
    draft.labels.push("contactGroups/work2".to_owned());
    let (fields, body) = draft.patch();
    assert_eq!(fields, ["memberships"]);
    let groups: Vec<_> = body["memberships"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            m["contactGroupMembership"]["contactGroupResourceName"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(groups, ["contactGroups/myContacts", "contactGroups/work2"]);
}

#[test]
fn a_contact_is_never_left_in_no_group() {
    let contact = Contact::parse(
        json!({ "resourceName": "people/a", "memberships": [
            { "contactGroupMembership": { "contactGroupResourceName": "contactGroups/1" } }
        ]}),
        &[],
    )
    .unwrap();
    let mut draft = Draft::new(&Login::extra("me@example.org"), &contact, false);
    draft.labels.clear();
    assert_eq!(
        draft.patch().1["memberships"],
        json!([{ "contactGroupMembership": { "contactGroupResourceName": "contactGroups/myContacts" } }])
    );
}

#[test]
fn a_directory_entry_copies_to_a_contact_without_googles_own_fields() {
    let draft = Draft::new(&Login::extra("me@example.org"), &contact(), true);
    let body = draft.as_new_contact();
    assert_eq!(body.get("metadata"), None);
    assert_eq!(body.get("memberships"), None);
    assert_eq!(body["names"][0].get("displayName"), None);
    assert_eq!(body["names"][0]["givenName"], "Ada");
    assert_eq!(body["emailAddresses"][0].get("metadata"), None);
}

#[test]
fn a_directory_entry_cannot_be_edited() {
    let mut state = book_state();
    let _ = update(&mut state, Action::Open(target("people/d1", true)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::GivenName, "Changed".to_owned()),
    );
    let draft = state.people.draft.as_ref().unwrap();
    assert!(draft.read_only);
    assert_ne!(draft.given, "Changed");
    assert!(!draft.dirty());
}

#[test]
fn unsaved_changes_are_not_lost_without_asking() {
    let mut state = book_state();
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::GivenName, "Augusta".to_owned()),
    );

    let _ = update(&mut state, Action::Close);
    assert_eq!(
        state.people.draft.as_ref().unwrap().pending,
        Some(Next::Close)
    );
    let _ = update(&mut state, Action::KeepEditing);
    let draft = state.people.draft.as_ref().unwrap();
    assert_eq!(draft.pending, None);
    assert_eq!(draft.given, "Augusta");

    let _ = update(&mut state, Action::Open(target("people/d1", true)));
    assert_eq!(
        state.people.draft.as_ref().unwrap().contact.resource,
        "people/c1"
    );
    let _ = update(&mut state, Action::Discard);
    assert_eq!(
        state.people.draft.as_ref().unwrap().contact.resource,
        "people/d1"
    );

    let _ = update(&mut state, Action::Close);
    assert!(state.people.draft.is_none());
}

#[test]
fn a_saved_contact_updates_the_list_and_the_panel() {
    let mut state = book_state();
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::GivenName, "Augusta".to_owned()),
    );
    let mut saved = person();
    saved["names"][0]["displayName"] = json!("Augusta Lovelace");
    saved["names"][0]["givenName"] = json!("Augusta");
    saved["etag"] = json!("newer");
    let _ = update(
        &mut state,
        Action::Saved("me@example.org".to_owned(), Ok(saved)),
    );
    assert_eq!(state.people.books[0].contacts[0].name, "Augusta Lovelace");
    let draft = state.people.draft.as_ref().unwrap();
    assert!(draft.saved);
    assert!(!draft.dirty());
    assert_eq!(draft.patch().1["etag"], "newer");
}

#[test]
fn a_failed_save_keeps_the_edits_and_says_why() {
    let mut state = book_state();
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::GivenName, "Augusta".to_owned()),
    );
    let problem = Problem::sign_in("expired");
    let _ = update(
        &mut state,
        Action::Saved("me@example.org".to_owned(), Err(problem.clone())),
    );
    let draft = state.people.draft.as_ref().unwrap();
    assert_eq!(draft.problem, Some(problem));
    assert_eq!(draft.given, "Augusta");
    assert!(draft.dirty());
}

#[test]
fn a_directory_entry_added_to_contacts_opens_as_an_editable_contact() {
    let mut state = book_state();
    let _ = update(&mut state, Action::Open(target("people/d1", true)));
    let created =
        json!({ "resourceName": "people/c2", "names": [{"displayName": "Grace Hopper"}] });
    let _ = update(
        &mut state,
        Action::Added("me@example.org".to_owned(), Ok(created)),
    );
    let draft = state.people.draft.as_ref().unwrap();
    assert!(!draft.read_only);
    assert_eq!(draft.contact.resource, "people/c2");
    assert_eq!(state.people.books[0].contacts.len(), 2);
}

#[test]
fn a_label_that_exists_is_chosen_rather_than_made_again() {
    let mut state = book_state();
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::NewLabel, " work ".to_owned()),
    );
    let _ = update(&mut state, Action::NewLabelSubmitted);
    let draft = state.people.draft.as_ref().unwrap();
    assert!(!draft.creating_label);
    assert!(draft.new_label.is_empty());
    assert!(
        draft
            .labels
            .iter()
            .any(|label| label == "contactGroups/work2")
    );
    assert_eq!(state.people.books[0].groups.len(), 2);
}

#[test]
fn stale_results_are_ignored() {
    let mut state = book_state();
    state.people.generation = 5;
    state.people.books[0].loading = true;
    let _ = update(
        &mut state,
        Action::Loaded(4, "me@example.org".to_owned(), Ok((vec![], vec![]))),
    );
    assert_eq!(state.people.books[0].contacts.len(), 1);
    assert!(state.people.books[0].loading);

    state.people.search_generation = 3;
    let _ = update(
        &mut state,
        Action::DirectoryFound(2, "me@example.org".to_owned(), Ok(api::Directory::None)),
    );
    assert_eq!(state.people.books[0].directory.len(), 1);
}

#[test]
fn clearing_the_search_clears_the_directory_results() {
    let mut state = book_state();
    let _ = update(&mut state, Action::QueryChanged("gr".to_owned()));
    assert_eq!(state.people.books[0].directory.len(), 1);
    let _ = update(&mut state, Action::QueryChanged("g".to_owned()));
    assert!(state.people.books[0].directory.is_empty());
}

#[test]
fn a_personal_account_stops_offering_a_directory_once_google_says_so() {
    let mut state = book_state();
    assert_eq!(
        state.people.books[0].directory_domain(),
        Some("example.org")
    );
    state.people.search_generation = 1;
    let _ = update(
        &mut state,
        Action::DirectoryFound(1, "me@example.org".to_owned(), Ok(api::Directory::None)),
    );
    assert_eq!(state.people.books[0].directory_domain(), None);
}

#[test]
fn choosing_the_contact_already_open_keeps_its_edits() {
    let mut state = book_state();
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::GivenName, "Augusta".to_owned()),
    );
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let draft = state.people.draft.as_ref().unwrap();
    assert_eq!(draft.pending, None);
    assert_eq!(draft.given, "Augusta");
}

/// A state whose cache is a real file in a temporary directory.
fn cached_state() -> (State, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("people/contacts.json");
    let mut state = book_state();
    state.people.cache_path = Some(path.clone());
    (state, dir, path)
}

/// Writes happen on a scratch thread, so wait for the file to say what's wanted.
fn wait_for_cache(path: &std::path::Path, wanted: impl Fn(&[Book]) -> bool) -> Vec<Book> {
    for _ in 0..500 {
        let books = cache::load(path);
        if wanted(&books) {
            return books;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the cache never reached the expected state");
}

fn account() -> String {
    "me@example.org".to_owned()
}

#[test]
fn every_contact_fetched_is_cached_and_read_back() {
    let (mut state, _dir, path) = cached_state();
    state.people.generation = 1;
    let groups = vec![group("friends1", "Book club")];
    let other = json!({ "resourceName": "people/c9", "names": [{"displayName": "Alan Turing"}] });
    let _ = update(
        &mut state,
        Action::Loaded(1, account(), Ok((vec![person(), other], groups))),
    );

    let books = wait_for_cache(&path, |books| {
        books.first().is_some_and(|b| b.contacts.len() == 2)
    });
    assert_eq!(books[0].account(), "me@example.org");
    assert_eq!(books[0].contacts[0].name, "Ada Lovelace");
    assert_eq!(books[0].contacts[0].labels, ["Book club"]);
    assert_eq!(books[0].contacts[0].person, person());
    assert_eq!(books[0].contacts[1].name, "Alan Turing");
    assert!(!books[0].loading);
}

#[test]
fn the_cache_is_only_readable_by_this_user() {
    use std::os::unix::fs::PermissionsExt;
    let (mut state, _dir, path) = cached_state();
    state.people.generation = 1;
    let _ = update(
        &mut state,
        Action::Loaded(1, account(), Ok((vec![person()], vec![]))),
    );
    wait_for_cache(&path, |books| !books.is_empty());
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0);
}

#[test]
fn an_edit_is_cached_as_google_returned_it_not_as_it_was_typed() {
    let (mut state, _dir, path) = cached_state();
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::GivenName, "augusta".to_owned()),
    );
    // Google tidies what it is given, and issues a new etag.
    let mut returned = person();
    returned["names"][0]["displayName"] = json!("Augusta Lovelace");
    returned["names"][0]["givenName"] = json!("Augusta");
    returned["etag"] = json!("newer");
    let _ = update(&mut state, Action::Saved(account(), Ok(returned.clone())));

    let books = wait_for_cache(&path, |books| {
        books
            .first()
            .is_some_and(|b| b.contacts[0].name == "Augusta Lovelace")
    });
    assert_eq!(books[0].contacts[0].person, returned);
    assert_eq!(state.people.draft.as_ref().unwrap().given, "Augusta");
}

#[test]
fn a_failed_edit_leaves_the_cache_alone() {
    let (mut state, _dir, path) = cached_state();
    let _ = update(&mut state, Action::Open(target("people/c1", false)));
    let _ = update(
        &mut state,
        Action::FieldEdited(Field::GivenName, "Augusta".to_owned()),
    );
    let _ = update(
        &mut state,
        Action::Saved(account(), Err(Problem::sign_in("expired"))),
    );
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!path.exists());
}

#[test]
fn a_failed_refresh_keeps_the_cached_contacts_on_show() {
    let (mut state, _dir, path) = cached_state();
    state.people.generation = 1;
    let _ = update(
        &mut state,
        Action::Loaded(1, account(), Err(Problem::sign_in("expired"))),
    );
    assert_eq!(state.people.books[0].contacts.len(), 1);
    assert!(state.people.books[0].problem.is_some());
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!path.exists());
}

#[test]
fn directory_people_are_cached_and_found_again_without_google() {
    let (mut state, _dir, path) = cached_state();
    state.people.search_generation = 1;
    let hedy = json!({ "resourceName": "people/d2", "names": [{"displayName": "Hedy Lamarr"}] });
    let _ = update(
        &mut state,
        Action::DirectoryFound(1, account(), Ok(api::Directory::Found(vec![hedy]))),
    );
    // Google's answer is what is shown, and the earlier result is still known.
    assert_eq!(state.people.books[0].directory.len(), 1);
    assert_eq!(state.people.books[0].directory[0].name, "Hedy Lamarr");

    let mut books = wait_for_cache(&path, |books| {
        books.first().is_some_and(|b| b.directory_seen.len() == 2)
    });
    books[0].show_seen("grace");
    assert_eq!(books[0].directory[0].name, "Grace Hopper");

    let _ = update(&mut state, Action::QueryChanged("grace".to_owned()));
    assert_eq!(state.people.books[0].directory[0].name, "Grace Hopper");
}

#[test]
fn signing_an_account_out_removes_it_from_the_cache() {
    let (mut state, _dir, path) = cached_state();
    state
        .people
        .books
        .push(Book::for_tests(Login::primary("main@example.org")));
    let _ = update(&mut state, Action::Open(target("people/c1", false)));

    let _ = forget_account(&mut state, "ME@example.org");
    assert!(state.people.draft.is_none());
    let books = wait_for_cache(&path, |books| books.len() == 1);
    assert_eq!(books[0].account(), "main@example.org");
    let saved: serde_json::Value =
        crate::task_db::read_document(&path, crate::task_db::keys::CONTACTS)
            .unwrap()
            .unwrap();
    assert!(!saved.to_string().contains("Lovelace"));

    let _ = forget_primary(&mut state);
    wait_for_cache(&path, |books| books.is_empty());
    assert!(path.exists());
}

#[test]
fn an_account_no_longer_signed_in_leaves_the_cache_at_the_next_refresh() {
    let (mut state, _dir, path) = cached_state();
    state.people.generation = 1;
    let _ = update(&mut state, Action::Listed(1, Ok(vec![])));
    assert!(state.people.books.is_empty());
    wait_for_cache(&path, |books| books.is_empty() && path.exists());
}

#[test]
fn offline_the_calendar_account_keeps_its_cached_contacts() {
    let mut state = State::blank();
    let mut book = Book::for_tests(Login::primary("main@example.org"));
    book.contacts = vec![contact()];
    state.people.books.push(book);
    state.people.generation = 1;
    // Its address can't be looked up, so it arrives under the generic name.
    let unidentified = Login::primary("Google Calendar account");
    assert!(unidentified.is_unidentified());
    let _ = update(&mut state, Action::Listed(1, Ok(vec![unidentified])));
    assert_eq!(state.people.books[0].account(), "main@example.org");
    assert_eq!(state.people.books[0].contacts.len(), 1);
}

#[test]
fn a_different_calendar_account_does_not_inherit_the_cached_contacts() {
    let mut state = State::blank();
    let mut book = Book::for_tests(Login::primary("old@example.org"));
    book.contacts = vec![contact()];
    state.people.books.push(book);
    state.people.generation = 1;
    let _ = update(
        &mut state,
        Action::Listed(1, Ok(vec![Login::primary("new@example.org")])),
    );
    assert_eq!(state.people.books[0].account(), "new@example.org");
    assert!(state.people.books[0].contacts.is_empty());
}

#[test]
fn not_learning_who_is_signed_in_drops_nobody() {
    let (mut state, _dir, _path) = cached_state();
    state.people.generation = 1;
    let _ = update(&mut state, Action::Listed(1, Err("worker died".to_owned())));
    assert_eq!(state.people.books.len(), 1);
    assert!(!state.people.listing);
}

#[test]
fn the_newest_snapshot_wins_whatever_order_they_are_written_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite3");
    let mut state = book_state();
    let older = cache::snapshot(&state.people.books).unwrap();
    state.people.books[0].contacts.clear();
    let newer = cache::snapshot(&state.people.books).unwrap();
    newer.write(&path).unwrap();
    older.write(&path).unwrap();
    assert!(cache::load(&path)[0].contacts.is_empty());
}

#[test]
fn a_damaged_cache_is_the_same_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite3");
    assert!(cache::load(&path).is_empty());
    crate::task_db::write_document_json(&path, crate::task_db::keys::CONTACTS, "not json").unwrap();
    assert!(cache::load(&path).is_empty());
}

#[test]
fn starting_up_shows_the_cached_contacts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite3");
    let state = book_state();
    cache::snapshot(&state.people.books)
        .unwrap()
        .write(&path)
        .unwrap();
    let mut store = Store::default();
    store.go_live(Some(path));
    assert_eq!(store.books[0].contacts[0].name, "Ada Lovelace");
    assert!(!store.books[0].loading);
}

#[test]
fn guests_are_found_once_across_accounts_and_can_be_left_out() {
    let mut state = store_with_book_for_guests();
    let other = state.books[0].clone();
    state.books.push(other);
    // One suggestion per address, however many accounts hold the contact.
    let found = state.invitees("ada", &[]);
    let emails = contact().emails;
    assert!(emails.len() > 1);
    assert_eq!(found.len(), emails.len());
    assert!(found.iter().all(|person| person.name == "Ada Lovelace"));
    let taken: Vec<String> = emails.iter().map(|email| email.to_uppercase()).collect();
    assert!(state.invitees("ada", &taken).is_empty());
    assert!(state.invitees("a", &[]).is_empty());
}

fn store_with_book_for_guests() -> Store {
    let mut book = Book::for_tests(Login::extra("me@example.com"));
    book.contacts = vec![contact()];
    Store {
        books: vec![book],
        ..Store::default()
    }
}
