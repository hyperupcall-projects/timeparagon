# Notes

Notes are Markdown documents listed under **More → Notes**. They are separate
from the notepad behind the **Note** tab.
They replace the old Google Keep integration, which has been removed.

## Storage

Each note is a Markdown file, `<title>.md`, in the directory set in
`config.toml`:

```toml
[notes]
directory = "~/Documents/Notes"
```

The setting is empty by default, and notes are not saved until it is set.

- Editing a note in the app saves it to its file. Renaming the note renames the
  file, and deleting the note deletes it.
- Characters that cannot be in a file name (`/ \ : * ? " < > |`) become `-`. An
  untitled note is `Untitled.md`. Notes with the same title are told apart as
  `Title 2.md`, `Title 3.md` and so on.
- A note's color (unless it is the default) and its connections are kept in
  front matter at the top of the file, which other Markdown editors such as
  Obsidian show as properties. Other front matter, such as tags, is kept.
- Files changed, added, renamed or removed outside the app show up within a
  couple of seconds. Other files in the directory are left alone.
- Notes are not stored in the database or synced through Turso. To share them
  between devices, sync the directory itself (with Syncthing, say).
- Older builds kept notes in `<data dir>/timeslay.sqlite3`. On the first start
  with a directory set, they are written to the directory and removed from the
  database, including their sync history, which is also deleted from the sync
  server.
- A new, empty note is dropped when closed.

## Editing

Notes have **Edit** and **View** tabs. View supports:

- Headings
- Bullet, numbered and check lists
- Bold, italics and strikethrough
- Quotes, code and horizontal rules
- Web and mail links

Anything else, such as tables or HTML, shows as plain text.

## Connections

Notes connect to events, time blocks, tasks and habits.

- Events, time blocks and tasks show their notes in their details panes. A
  note can be created or connected from there.
- A habit's page shows its notes. A note attaches to the habit in
  **More → Habits**, not a calendar event, so it shows on every event series
  connected to that habit.
- A habit event with no habit chosen cannot have notes.
- A note on a recurring event or time block attaches to the one occurrence it
  was connected from, not the whole series.
- Searching from inside a note finds all unarchived habits, but only the events,
  time blocks and tasks already loaded within 30 days of the day in view. For
  anything else, connect from its details pane.

## Colors

Each note has one of 8 light pastel backgrounds: Butter (the default), Peach,
Rose, Lilac, Lavender, Sky, Aqua or Mint. There is no "no color" option. Text on
a note is always dark, on every theme.

## Deleting and conflicts

- Deleting a note asks for confirmation, then deletes its file.
- An edit made in the app while the file was changed elsewhere overwrites that
  change.
- Notes are available in both the desktop and Android apps, each reading the
  directory in its own `config.toml`. Saving does not depend on Google sign-in.

## Upgrading from Google Keep

Keep notes are not migrated. The removed integration can leave data on disk,
which is not deleted automatically:

- The helper's Chrome profile, which may hold a Google session. It is probably
  at `~/.local/share/timeslay/keep-bridge/chrome-profile`.
- The old cache at `~/.cache/timeslay/keep`.

The installer and uninstaller remove the old `keep-bridge` binary.
