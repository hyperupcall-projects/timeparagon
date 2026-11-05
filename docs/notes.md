# Notes

Notes are Markdown documents listed under **More → Notes**. They are separate
from the notepad behind the **Note** tab, which is one page in the config file.
They replace the old Google Keep integration, which has been removed.

## Storage

- Each note is one `.md` file in the hidden Google Drive app folder. This uses
  the existing `drive.appdata` permission, so nobody needs to reconnect Google.
- The note's title is the filename. Its color and connections are JSON in the
  file's `description`, so the file itself stays plain Markdown.
- Notes save automatically and work offline from a copy in the local database
  (`<data dir>/timeslay.sqlite3`). They upload about 3 seconds after typing stops.
- Notes sync when the window opens, when the Notes screen opens, and on the
  calendar's periodic sync.
- A new note stays off Drive until it has a title, text, or a connection. An
  empty one is dropped when closed.

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
Rose, Lilac, Lavender, Sky, Aqua or Mint. There is no "no color" option, and an
unknown color read from Drive falls back to the default. Text on a note is
always dark, on every theme.

## Deleting and conflicts

- Deleting a note asks for confirmation, then permanently deletes the Drive file.
- If two devices change the same note, Drive's version stays and the local edits
  become a separate "Title (conflict)" note, so no text is lost.
- If a note is deleted on another device while it has unsynced edits here, those
  edits are uploaded as a new file.

## Limitations

- Notes are desktop only. The Android app has no Notes screen.
- Sync is covered by tests against a fake Drive. It has not yet been exercised
  against real Google Drive.

## Upgrading from Google Keep

Keep notes are not migrated. The removed integration can leave data on disk,
which is not deleted automatically:

- The helper's Chrome profile, which may hold a Google session. It is probably
  at `~/.local/share/timeslay/keep-bridge/chrome-profile`.
- The old cache at `~/.cache/timeslay/keep`.

The installer and uninstaller remove the old `keep-bridge` binary.
