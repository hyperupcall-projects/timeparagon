//! Moves the files older builds kept, and the data `config.toml` held, into the
//! local database, then reports what the app can read back. The app does the
//! same move when it starts, so this is only for doing it without opening the
//! app:
//!
//! ```sh
//! cargo run -p timeslay-migrate --example migrate_local_data
//! ```

use timeslay::calendar::{Store, oauth};
use timeslay::habits::Library;
use timeslay::task_db::{self, keys};

fn main() {
    let Some(path) = task_db::path() else {
        eprintln!("this device has no data directory");
        std::process::exit(1);
    };
    println!("database: {}", path.display());
    println!("signed in to Google: {}", oauth::load_tokens().is_some());
    match task_db::read_document::<Library>(&path, keys::HABITS) {
        Ok(library) => {
            let count = library.map_or(0, |library| library.habits.len());
            println!("habits: {count}");
        }
        Err(error) => println!("habits could not be read: {error}"),
    }
    match task_db::read_document::<serde_json::Value>(&path, keys::NOTES) {
        Ok(notes) => {
            let count = notes
                .as_ref()
                .and_then(|notes| notes["notes"].as_array())
                .map_or(0, Vec::len);
            println!("notes: {count}");
        }
        Err(error) => println!("notes could not be read: {error}"),
    }
    let config = timeslay::config::Config::load();
    println!(
        "todos: {}, notepad characters: {}, brain dump pages: {}, projects: {}, work sessions: {}",
        config.todos.len(),
        config.notepad.text.chars().count(),
        config.brain_dump.entries.len(),
        config.projects.items.len(),
        config.task_work.sessions.len()
    );
    let mut store = Store::new(0);
    store.cache_days_in(Some(path));
    println!(
        "calendars: {}, cached events today: {}",
        store.calendars.len(),
        store.events.len()
    );
}
