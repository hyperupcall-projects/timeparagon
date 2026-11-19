# Notes

Notes are Markdown documents listed under **More → Notes**. They are separate
from **More → Notepad**, which is also written in Markdown and
has the same **Edit** and **View** tabs.

## Storage

Each note is a Markdown file, `<title>.md`. Choose its folder under
**Settings → Notes** on desktop and mobile. On Android, **Choose folder**
opens the system folder picker and keeps access across restarts. Android
requires this picker to grant access; typing a filesystem path does not grant
storage permissions. The folder
is a device-local setting; choose the corresponding synced folder on each
device to see the same notes. Switching folders saves pending edits in the
previous folder first. Notes drafted before setup are saved in the chosen folder.

It is the `notes.directory` row of `device_settings` in the database (see
[Settings](./data-model.md#settings)), which never leaves the device:

```sh
sqlite3 ~/.local/share/timeparagon/timeparagon.sqlite3 \
  "SELECT value FROM device_settings WHERE key = 'notes.directory'"
```

The setting is empty by default, and notes are not saved until it is set.

- Editing a note in the app saves it to its file. Renaming the note renames the
  file, and deleting the note deletes it.
- Characters that cannot be in a file name (`/ \ : * ? " < > |`) become `-`. An
  untitled note is `Untitled.md`. Notes with the same title are told apart as
  `Title 2.md`, `Title 3.md` and so on.
- Front matter contains a stable `id`, so renaming a file in another editor
  preserves its identity. Colors and connections are stored in the database's
  `notes` table. All other front matter, including unused properties, is removed.
- Existing notes are migrated automatically when loaded: their color and
  connections are committed to the database before those properties are removed
  from the file. Notes without IDs receive one.
- Files changed, added, renamed or removed outside the app show up within a
  couple of seconds. Other files in the directory are left alone.
- Note text stays in Markdown. To share notes between devices, sync the
  directory itself (with Syncthing, say) and enable database sync for colors and
  connections. Metadata follows each note's ID across folders and devices.
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
- In an event's or time block's details pane, connecting and disconnecting
  notes wait for **Save**, like its other fields. On a recurring one, **Save
  current event** applies to that occurrence, **Save current and future
  events** to it and every later one, and **Save all events** to every
  occurrence. Disconnecting one occurrence, or it and later ones, leaves the
  rest of the series connected.
- **New note** puts the note on its occurrence straight away, so it is never
  left unconnected. Saving with a wider scope extends it.
- A save that only connects or disconnects notes writes nothing to Google
  Calendar, so it also works on calendars that can't be edited.
- Connecting from inside a note connects the one occurrence found.
- Retiming a series or editing "this and following" keeps its notes on the
  same occurrences. Existing connections made with earlier versions are
  recognized too.
- Searching from inside a note finds all unarchived habits, but only the events,
  time blocks and tasks already loaded within 30 days of the day in view. For
  anything else, connect from its details pane.

## Colors

Each note has one of 8 light pastel backgrounds: Butter (the default), Peach,
Rose, Lilac, Lavender, Sky, Aqua or Mint. There is no "no color" option. Text on
a note is always dark, on every theme.

## Deleting and conflicts

- Deleting a note asks for confirmation, then deletes its file and database
  metadata. Metadata for files removed outside the app is retained, so a file
  temporarily absent during folder sync does not lose its connections.
- An edit made in the app while the file was changed elsewhere overwrites that
  change.
- Notes are available in both the desktop and Android apps. Saving does not
  depend on Google sign-in.
- **Open in default app** saves the latest edits and opens the actual Markdown
  file with the system's associated application. On Android, the app receives
  access to that document. Changes made there appear after returning to TimeParagon.
  An installed app that handles Markdown or plain text is needed.
