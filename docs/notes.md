# Notes

Notes are Markdown documents listed under **More → Notes**. They are separate
from the notepad behind the **Note** tab.
They replace the old Google Keep integration, which has been removed.

## Storage

- Notes save automatically to `<data dir>/timeslay.sqlite3` and work offline.
- Turso synchronizes them through the private Tailscale server alongside habits
  and scratch data. Both devices must be connected to Tailscale to exchange edits.
- Existing local notes and their links are retained. The app no longer accesses
  Drive; files previously stored there are left intact.
- Fields merge independently, and every version is retained in the sync journal
  for recovery if two devices edit the same text. Export versions with the
  `database_history` example in `timeslay-core`.
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

- Deleting a note asks for confirmation, then removes it from the local library.
  The database journal replicates that deletion to other devices.
- Fields merge independently across devices. Conflicting text revisions remain
  in the sync journal and can be exported with the `database_history` example.
- Notes are available in both the desktop and Android apps. Saving and sync do
  not depend on Google sign-in.

## Upgrading from Google Keep

Keep notes are not migrated. The removed integration can leave data on disk,
which is not deleted automatically:

- The helper's Chrome profile, which may hold a Google session. It is probably
  at `~/.local/share/timeslay/keep-bridge/chrome-profile`.
- The old cache at `~/.cache/timeslay/keep`.

The installer and uninstaller remove the old `keep-bridge` binary.
