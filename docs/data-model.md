# Data model

This document covers what TimeSlay stores, where each piece lives, how the
pieces refer to each other, and the exact shape each one has on disk or on
Google. The SQLite tables are described column by column in the
[database schema](./database-schema.md).

## Where data lives

TimeSlay is local-first, and it has no backend of its own. Data is kept in
whichever store suits it best:

```mermaid
flowchart TB
    subgraph Google
        GC[Google Calendar<br/>events, time blocks, dated tasks,<br/>habit events, sleep]
        GT[Google Tasks<br/>undated tasks]
        GP[Google People<br/>contacts, directory]
    end
    subgraph Device["Each device"]
        DB[(timeslay.sqlite3)]
        CFG[config.toml]
        SEC[secrets.json /<br/>Android Keystore blob]
        NOTES[Notes directory<br/>*.md]
    end
    TURSO[(Turso sync server)]
    SYNCTHING[[Syncthing or similar]]

    GC <-- "immediate writes + refresh" --> DB
    GT <-- "immediate writes + refresh" --> DB
    GP <-- "cache" --> DB
    DB <-- "6 shared documents" --> TURSO
    NOTES <-. "you sync it" .-> SYNCTHING
```

| Store | Owns | Shared between devices by |
| --- | --- | --- |
| **Google Calendar** | Events, time blocks, scheduled (dated) tasks, habit occurrences, sleep nights, meeting guests and replies | Google |
| **Google Tasks** | Undated tasks (inbox, triaged, backlog) | Google |
| **Google People** | Contacts, contact groups, Workspace directory | Google |
| **SQLite** `timeslay.sqlite3` | Habits, alarms, notepad, brain dump, projects and life areas, work sessions; caches of everything Google owns; local copies of edits being sent to Google | Turso, for six documents only |
| **`config.toml`** | Device settings: calendar choices, colors, views, window, panel, voice, places and travel, pomodoro, reminders, sync URL, Google OAuth client | Not shared. Copy it by hand if you want to. |
| **`secrets.json`** | Google tokens, connected Gmail accounts, OpenRouter token | Not shared |
| **Notes directory** | Markdown notes, one file each | Whatever syncs the folder (Syncthing, say) |

Google is authoritative for everything it stores. SQLite holds a cache of that
data so the app starts instantly and can be read offline. Changes are sent to
Google straight away; the changed item shows greyed out and locked until Google
accepts it, and a refused change is undone and logged. There is no queue, so
changes cannot be made offline (see
[Google writes](./database-schema.md#google-writes)).

### File locations

Paths use the XDG directories for app ID `dev.TimeSlay.timeslay`:

| File | Linux |
| --- | --- |
| Config | `~/.config/timeslay/config.toml` |
| Database | `~/.local/share/timeslay/timeslay.sqlite3` |
| Turso replica | `~/.local/share/timeslay/turso/replica.db` |
| Secrets | `~/.local/state/timeslay/secrets.json` (mode `0600`) |
| IPC sockets | `$XDG_RUNTIME_DIR/timeslay-sidebar.sock`, `timeslay-dock.sock` |

On Android the database is at `<files dir>/data/timeslay/timeslay.sqlite3`.
Google tokens are encrypted with an AES key held in Android Keystore
(alias `dev.timeslay.google-tokens.v1`) and stored in the no-backup directory.
The notes folder is chosen with the system folder picker.

Test runs (`config::isolated()`) redirect every one of these paths to a
scratch directory and never contact Google or the sync server.

### What is shared between devices

Turso replicates exactly these `documents` rows (`database_sync::SHARED_KEYS`):

| Document | Holds |
| --- | --- |
| `habits` | [Habits](#habits) |
| `alarms` | [Alarms](#alarms) |
| `shared.notepad` | [Notepad](#notepad-and-brain-dump) text |
| `shared.brain_dump` | [Brain dump](#notepad-and-brain-dump) pages |
| `shared.projects` | [Projects, life areas, goals, objectives](#life-areas-goals-objectives-and-projects) |
| `shared.task_work` | [Work sessions and distractions](#work-sessions-distractions-and-pomodoro) |

Everything else stays on the device. That includes the Google caches and the
local copies of edits being sent (each device talks to Google itself), contacts, cached routes,
calendar sync state, the Android theme, all of `config.toml`, secrets and
notes.

Concurrent edits are merged **field by field**, not by whichever document was
saved last. See [Merge rules](./database-schema.md#merge-rules). This is why
every list item in a shared document has a stable identity field.

---

## Identifiers

Many kinds of ID appear in the data. This table lists them all:

| ID | Format | Example | Defined in |
| --- | --- | --- | --- |
| Google calendar ID | Google's | `abc123@group.calendar.google.com`, `primary` | Google |
| Task-list "calendar" ID | `tasks:` + list ID | `tasks:MDk2NjQ…` | `calendar::ids::calendar_id` |
| Google event ID | Google's (base32hex) | `5ok3…_20261109T090000Z` | Google |
| Google task "event" ID | `task:` + task ID | `task:dGhpcyBp…` | `calendar::ids::event_id` |
| Dated task event ID | `tt` + 32 hex | `tt3f9a…` | `calendar_tasks::apply` |
| Sleep event ID | `51ee9` + `YYYYMMDD` (wake date) | `51ee920261110` | `calendar::sleep::event_id` |
| **TimeSlay task ID** | integer ≥ 100, < 2⁶² | `2875093487120931` | `task_db::Database::import` |
| Block key | `<calendar_id>/<event_id>` | `abc@group…/5ok3…` | `timeblocking::block_key` |
| Task key | task ID, or `<task id>/<event id>` for an occurrence of a recurring dated task | `2875…` | `timeblocking::task_key` |
| Habit ID | string | `h1`, random | `habits::Habit::id` |
| Habit section ID | string | | `habits::Section::id` |
| Alarm ID | random `u64` | | `alarms::new_id` |
| Project / life area / goal / objective / place ID | `u64` | Personal area is always `1` | `config`, `places` |
| Note | its file name (`<title>.md`) | `Groceries.md` | `notes::files` |
| Contact | People API resource name | `people/c123…` | Google |
| Edited item (`google_entities.id`) | `<calendar_id>\n<event_id>` | | `operations::key` |
| Sync edit | 32 random hex chars, or `snapshot-` + 32 hex | | `database_sync` |

Names are never used as references. Tasks refer to projects, places and
time blocks by ID, so you can rename those freely.

---

## Calendar

### Calendar

`calendar::Calendar` is one source shown in the calendar list:

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Google calendar ID, or `tasks:<listId>` for a Google Tasks list |
| `name` | string | |
| `color` | color | Google's color, which `config.toml` can override |
| `primary` | bool | The account's primary calendar |
| `selected_by_google` | bool | Checked in Google Calendar's own list |

Google Tasks lists are shown as **synthetic calendars** whose IDs start with
`tasks:`. Three list names are reserved for the task workflow and are not
shown as calendar sources: `Inbox`, `Triaged` and `Backlog` (or the lists
configured as `inbox_list`, `triaged_list` and `backlog_list`).

The **app calendar** is the one Google calendar TimeSlay writes its own items
to. It is named `TimeSlay` and its ID is recorded as `[calendar] app_calendar`.
It holds time blocks, all-day app events, dated tasks, habit occurrences and
sleep.

The catalog of calendars and the time of the last successful refresh are cached
in the `calendar_sync_state` document, so a restart can show source names and
honor the sync interval.

### Event

`calendar::Event` is the single type for everything placed on the calendar,
including Google Tasks:

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | See [Identifiers](#identifiers) |
| `calendar_id` | string | |
| `title` | string | `(No title)` when Google's summary is empty |
| `start`, `end` | datetime (local) | All-day `end` is exclusive, as in Google |
| `all_day` | bool | |
| `color` | color | Already resolved for display |
| `completed` | bool | Completed or ignored task, or done habit occurrence |
| `details` | `Details` | Everything below |

`calendar::Details`:

| Field | Type | Source on Google |
| --- | --- | --- |
| `description` | string | Event `description` (HTML converted to plain text) or the prose part of a task's notes |
| `location` | string | `location` |
| `status` | string | `Confirmed` / `Tentative` / `Cancelled` |
| `link` | string | `htmlLink` |
| `meeting` | string | `hangoutLink`, else the first video entry point of `conferenceData` |
| `organizer` | string | |
| `attendees` | string[] | Display labels |
| `attendance` | `meeting::Attendance` | Guests as data, see [Meetings](#meetings) |
| `recurrence` | string[] | Raw RFC 5545 lines (`RRULE:`, `RDATE:`, `EXDATE:`) |
| `recurring_event_id` | string? | The series this occurrence belongs to |
| `color_id` | string? | Google `colorId`. `None` means the calendar's color |
| `options` | `EventOptions` | Reminders, availability, visibility, inherited child metadata |
| `projects` | `u64[]` | `ttProjects` |
| `labels` | string[] | `ttLabels` |
| `habit` | bool | `ttKind = "habit"` |
| `habit_id` | string? | `ttHabit` |
| `habit_completed_at` | datetime? | `ttCompletedAt` |
| `sleep` | `sleep::Record`? | `ttKind = "sleep"` + `ttSleep` |
| `task_data` | `TaskData`? | A task's TimeSlay ID and metadata, see [Tasks](#tasks) |
| `task_updated_at` | datetime? | Google Tasks `updated` |

`EventOptions`:

| Field | Type | Google field |
| --- | --- | --- |
| `reminders` | `{useDefault, overrides: [{method, minutes}]}` | `reminders` (`method`: `popup` / `email`; at most 5 overrides) |
| `default_reminders` | `Reminder[]` | The calendar's defaults from `events.list` (read only) |
| `transparency` | `opaque` (Busy) / `transparent` (Free) | `transparency` |
| `visibility` | `default` / `public` / `private` / `confidential` | `visibility` |
| `child_metadata` | `ChildMetadata` | `ttChildMetadata`, see [Time blocks](#time-blocks) |

### What kind of item an event is

Google only knows about events and tasks. TimeSlay works out its own kinds
from the metadata (`Event::kind`), checking these rules in order:

| Kind | Rule | Chip |
| --- | --- | --- |
| `Sleep` | `details.sleep` is set | Sleep |
| `Habit` | `details.habit` | Habit |
| `ScheduledTask` | It is a task, but not on a `tasks:` list (a dated task on the app calendar) | Task |
| `Task` | `task_data` is set, or the ID starts with `task:`, or the calendar ID starts with `tasks:` | Task |
| `TimeBlock` | On the app calendar and timed | Time Block |
| `AppEvent` | On the app calendar and all-day | Event |
| `External` | Anything else | Event |

### Google extended properties

TimeSlay stores its own data in **private** extended properties
(`extendedProperties.private`). These are only visible to the account that
wrote them. All values are strings, and Google limits each one to 1024
characters.

| Property | On | Value |
| --- | --- | --- |
| `ttKind` | Dated tasks, habit occurrences, sleep | `"task"`, `"habit"` or `"sleep"` |
| `ttData0` … `ttDataN` | Dated tasks | Chunks of the JSON-encoded `TaskData`, at most 900 bytes each |
| `ttDataCount` | Dated tasks | Number of chunks, `"1"`–`"200"` (at most 28 000 bytes in total) |
| `ttCompleted` | Dated tasks, habit occurrences | `"true"` / `"false"` |
| `ttCompletedAt` | Habit occurrences | RFC 3339 time it was marked done, or `null` |
| `ttHabit` | Habit occurrences | The [habit](#habits) ID it counts towards |
| `ttProjects` | Any event | JSON array of project IDs, e.g. `"[7,3]"` |
| `ttLabels` | Any event | JSON array of strings, e.g. `"[\"work\"]"` |
| `ttChildMetadata` | Time blocks | JSON `ChildMetadata` |
| `ttSleep` | Sleep | JSON `sleep::Record` |
| `ttSleepDate` | Sleep | Wake date `YYYY-MM-DD`, used to look the night up with `privateExtendedProperty` |

When TimeSlay patches an event it keeps any extended properties it does not
know about.

---

## Tasks

A task lives in **Google Tasks** while it has no date, and as a **Google
Calendar event on the app calendar** once it is scheduled
(`calendar_tasks.rs`: "Scheduled tasks are Calendar events. Google Tasks holds
only undated work."). Moving between the two changes the task's Google ID.
Its TimeSlay ID stays the same.

```mermaid
stateDiagram-v2
    [*] --> Inbox: capture (text, voice, Slack, browser, Gmail)
    Inbox --> Triaged: Triaged (estimate/project/priority filled in)
    Inbox --> Backlog: Backlog(section)
    Triaged --> Backlog
    Backlog --> Triaged
    Inbox --> Scheduled: Day / Time / Block
    Triaged --> Scheduled
    Backlog --> Scheduled
    Scheduled --> Inbox: Inbox
    Scheduled --> Backlog: Backlog / ClearSchedule
    Scheduled --> Scheduled: Time / Block / Reorder / Reflow / Unassign

    state "Google Tasks (undated)" as GTasks {
        Inbox
        Triaged
        Backlog
    }
    state "Google Calendar, app calendar (dated)" as GCal {
        Scheduled
    }
```

| Where | Representation | Where metadata lives |
| --- | --- | --- |
| Undated: `Inbox` / `Triaged` / `Backlog` list | Google Task. `Event.id = task:<id>`, `calendar_id = tasks:<list>`. Internally its date is the sentinel `1970-01-01` (`timeblocking::UNDATED`) | Footer of the task's `notes` |
| Dated, all-day ("do on this day") | All-day Calendar event, `ttKind = task` | `ttData*` properties |
| Dated, timed or in a time block | Timed Calendar event, `ttKind = task` | `ttData*` properties |

Before Google has assigned an ID, a new undated task is shown with the
placeholder ID `task:tt<32-hex TimeSlay ID>` on a `tasks:local-inbox`,
`tasks:local-triaged` or `tasks:local-backlog` list. A new dated task is
placed on `local-calendar` until the app calendar is known.

### Task status

| Status | `Event.completed` | `metadata.ignored` | Google Tasks `status` | Calendar `ttCompleted` |
| --- | --- | --- | --- | --- |
| Active | `false` | `false` | `needsAction` | `"false"` |
| Completed | `true` | `false` | `completed` | `"true"` |
| Ignored | `true` | `true` | `completed` | `"true"` |

Google has no "ignored" state. An ignored task counts as completed in Google,
and TimeSlay tells the two apart with the `ignored` flag.

A task is **overdue** if it is neither completed nor ignored and was due
before today. It is **inboxed** if it is still waiting to be organized.

### Task metadata

`task_db::TaskData` is `{ id, metadata }`. `id` is the TimeSlay task ID, and
`metadata` is `timeblocking::Metadata`. Unknown fields are rejected
(`deny_unknown_fields`), so an older build never overwrites data written by a
newer one.

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `minutes` | `u32` | required | Estimated duration. A timed task's length. `0` = no estimate (placed as 30 min) |
| `block` | string? | required (nullable) | Block key of the time block the task is assigned to |
| `order` | `i64` | `0` | Position within its block or day |
| `ignored` | bool | `false` | See [Task status](#task-status) |
| `backlog_section` | enum | `someday` | `next_week_or_two`, `next_month`, `next_quarter`, `next_year`, `someday`, `never`. A planning horizon. It does not change due dates |
| `deadline` | `{date, time?}`? | — | When the task must be finished. Without a time, the whole day counts |
| `priority` | `"P1"` / `"P2"`? | — | `P1` is the most urgent. Unset means no priority |
| `projects` | `u64[]` | `[]` | [Project](#life-areas-goals-objectives-and-projects) IDs |
| `labels` | string[] | `[]` | Free-form labels |
| `location` | `u64`? | — | [Place](#places-and-travel) ID |
| `created_at` | datetime (UTC)? | — | Set only for tasks created in TimeSlay |
| `repeat_source` | `i64`? | — | For a copy made by a recurring block, the task ID it was copied from. Cleared when the task leaves that block's series |
| `inherited` | `InheritedMetadata`? | — | Snapshot of the time block's settings the task is following, see [Time blocks](#time-blocks) |

Fields that are empty or `None` are left out when serializing.

#### Encoding in Google Tasks notes

An undated task stores its metadata after its notes, below a final `---`
line. Each field goes on its own line with a `tt-` prefix, and every value is
JSON:

```text
Pick up the dry cleaning before Friday.
---
tt-version: 1
tt-id: 2875093487120931
tt-deadline: {"date":"2026-11-13","time":null}
tt-backlog_section: "someday"
tt-block: null
tt-minutes: 15
tt-order: 0
tt-ignored: false
tt-projects: [7]
```

Rules (`calendar::task_metadata`):

- A footer only counts if it starts with `tt-version:`. A `---` the user typed
  is treated as ordinary text.
- `tt-version` must be `1`. `tt-id` must be ≥ 100. Duplicate keys or
  unknown keys are errors ("update the app before editing"). Malformed
  metadata is never overwritten.
- Notes and footer together must fit in Google Tasks' 8192-character limit.
- Everything before the footer is the task's description.

#### Encoding in Calendar events

A dated task stores the same `TaskData` as compact JSON, split into
`ttData0…ttDataN` with `ttDataCount = N+1` (see
[extended properties](#google-extended-properties)). `ttProjects` and `ttLabels`
are also written separately and **take precedence** over the copies inside
`ttData` when reading. The event's `description` is the task's description,
with no footer.

#### Local cache

SQLite caches every task's metadata in `tasks` and maps each Google identity
it has had to its TimeSlay ID in `task_aliases`. The full last-known task
`Event`s are kept in `google_task_records`. See [Tasks](./database-schema.md#tasks).

### Task edits

All task changes go through `task_edit::Change`. Each change also names the
write in the log (`Edit task Title`, `Edit task Block`, …):

| Change | Effect |
| --- | --- |
| `Title`, `Description` | Text |
| `Estimate(minutes)` | Sets `minutes`. Resizes a timed task |
| `Projects`, `Labels`, `Priority`, `Location`, `Deadline`, `BacklogSection` | Sets the field |
| `Day(date)` | All-day on that date. Leaves its block |
| `Time(start, end)` | Timed. Leaves its block. `minutes` = length |
| `Block(key, start, end)` | Assigns to a time block at a position |
| `Reorder(block?, order, day)` | Moves within or between blocks |
| `Reflow(start, end)` | Automatic re-layout. Membership and estimate unchanged |
| `Unassign` | Leaves its block but keeps its day |
| `Inbox`, `Triaged`, `Backlog(section?)`, `ClearSchedule` | Moves to an undated list |
| `Inherit(InheritedMetadata)`, `ReleaseInheritance` | Starts or stops following a block's settings |

Moving to another block or day also clears `repeat_source`.

---

## Time blocks

A time block is a **timed event on the app calendar** that is not a task,
habit or sleep. Tasks are assigned to it by `metadata.block = <block key>`
and placed within it by `order`.

A block can push settings onto its tasks. The block stores them in
`ttChildMetadata` as `ChildMetadata`:

| Field | Type | Meaning |
| --- | --- | --- |
| `force` | bool | Apply even over the task's own values |
| `due` | `Deadline`? | |
| `deadline` | `Deadline`? | |
| `priority` | `P1`/`P2`? | |
| `location` | `u64`? | Place ID |

When a task inherits from a block, the task records a full snapshot
(`InheritedMetadata`) in its own metadata:

| Field | Meaning |
| --- | --- |
| `series` | `<calendar>/<recurring event id>` for a recurring block |
| `fields` | The block's `ChildMetadata` |
| `projects`, `labels` | The block's |
| `recurrence` | The block's RRULE lines (copied to the task) |
| `options` | The block's `EventOptions` without `child_metadata` |

What a task inherits: projects, labels, priority, location, deadline,
recurrence and event options. What always stays the task's own: its identity,
which block it belongs to, its estimate, its work history and its completion.

Work done in a block is recorded in [work sessions](#work-sessions-distractions-and-pomodoro).

---

## Sleep

Each night is an ordinary Google event on the app calendar, with
`ttKind = "sleep"`. It has a stable ID derived from the wake date, so every
device writes the same event:

```json
// ttSleep
{"wake_date":"2026-11-10","automatic":true,"estimated_start":true,"estimated_end":true}
```

| Field | Meaning |
| --- | --- |
| `wake_date` | The night's identity. It does not change when the times are edited |
| `automatic` | Planned by TimeSlay rather than entered by hand |
| `estimated_start`, `estimated_end` | The time is still a plan, not a measurement |

The planner wakes you at the first timed event between `sleep_day_start_hour`
and `latest_wakeup_hour`, or at `latest_wakeup_hour` if there is none. Bedtime
is `sleep_goal_hours` (clamped to 6–12) earlier, counted in real elapsed time
so it stays correct across DST changes. All-day events and undated tasks do not
affect the wake time. The title (`Sleep · 8h`) and description are generated
again whenever the times change. Sleep is shown as its own source,
`timeslay:sleep`.

---

## Meetings

`meeting::Attendance` is derived from the Google event's `attendees` and guest
permissions:

| Field | Meaning |
| --- | --- |
| `guests` | `[{email, name, response, optional}]`. Resources and guests without an email are left out |
| `response` | `needsAction` / `accepted` / `declined` / `tentative` |
| `permissions` | `{modify, invite_others, see_guests}` (Google's `guestsCan*`; defaults `false`, `true`, `true`) |
| `organizer` | This copy of the event belongs to the organizer |
| `own_email` | The user's own address on the guest list |

Replying to an invitation is sent as a `Reply` job. Google Meet links come
from `hangoutLink`/`conferenceData`. Zoom meetings are made by opening the event
in the browser for the Zoom for Google Workspace add-on, because the Zoom API
is usually locked down for organization accounts.

---

## Habits

A habit is a goal with a day-by-day record, modeled on Loop Habit Tracker.
Scores and streaks are computed the same way Loop computes them. A habit does
not need a calendar event. Any number of events can count towards it through
`ttHabit`. Marking such an event done records one repetition, and marking it
not done takes one away.

Stored as the `habits` document (shared), type `habits::Library`:

```json
{
  "habits": [Habit, …],
  "sections": [{"id": "s1", "name": "Morning", "collapsed": false}],
  "sort": "manual",            // manual | name | color | score | status
  "reversed": false,
  "hide_archived": false,
  "hide_completed": false,
  "score_bucket": "day",       // day | week | month | quarter | year
  "history_bucket": "week"
}
```

`Habit`:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Stable identity. Events, objectives and notes refer to it |
| `name` | string | |
| `kind` | `yes` / `no` | `yes`: doing it is the goal. `no`: **avoiding** it is the goal, so every day it is not done is a success |
| `created` | date? | A `no` habit starts counting from here |
| `question` | string | e.g. "Did you meditate today?" |
| `notes` | string | |
| `color` | `u8` | Index into a 20-color palette (default 8) |
| `frequency` | `Frequency` | See below |
| `archived` | bool | |
| `skip_days` | bool | Clicking a done day offers "skipped" before "not done" |
| `question_marks` | bool | Days with no record show `?` instead of ✗ |
| `icons` | `symbols` / `faces` | |
| `marks` | `{date: Mark}` | The record. See below |
| `resisted` | `{date: u32}` | `no` habits: urges resisted that day. Does not affect the score |
| `log` | `{date: {happened: [LogEntry], resisted: [LogEntry]}}` | `no` habits: when each slip or resisted urge happened |
| `section` | string? | Section ID |
| `measure` | `{unit, target?, days}`? | Record a number each day. `days` is 1, 7 or 30 (the period the target covers). Never affects the score |
| `values` | `{date: i64}` | That number, in **thousandths**. Kept even if `measure` is turned off |
| `area` | `u64`? | Life area ID. Unset means Personal |

`Frequency` (externally tagged, snake case):

```json
{"interval": {"times": 3, "days": 7}}    // 3 times a week; {1,1} = daily (default)
{"times_per_day": {"times": 2}}          // more than once each day
{"weekdays": {"days": [true,false,true,false,true,false,false]}}  // Mon first
```

`Mark`: `"yes"`, `{"reps": 3}` (done 3 times that day), `"no"`, `"skip"`.
A day with no mark has no answer. `LogEntry` is `{at?: "HH:MM:SS", amount?:
thousandths, note?}`.

Statuses computed from the record (not stored): `Yes`, `Auto` (covered by an
earlier repetition), `Partial(n)`, `No`, `Skip`, `Unknown`, `Clean` /
`Slipped(n)` for `no` habits.

A habit's **calendar event** is a recurring event on the app calendar with
`ttKind = habit` and `ttHabit = <id>`. Each occurrence carries `ttCompleted` /
`ttCompletedAt`.

---

## Life areas, goals, objectives and projects

Stored as the `shared.projects` document (shared). In `config.toml` this is the
`[projects]` table (`config::ProjectsSettings`):

```mermaid
erDiagram
    LifeArea ||--o{ Goal : goals
    LifeArea ||--o{ Objective : objectives
    Goal |o--o{ Objective : "objective.goal"
    Objective }o--o{ Habit : "objective.habits"
    LifeArea |o--o{ Project : "project.area"
    LifeArea |o--o{ Habit : "habit.area"
    Project }o--o{ Task : "metadata.projects"
    Project }o--o{ Event : "ttProjects"
```

```json
{
  "items": [
    {"id": 7, "name": "School/Calculus", "color": "#4A90E2",
     "directory": "/home/me/school/calc", "area": 3}
  ],
  "areas": [
    {"id": 1, "name": "Personal", "color": null, "vision": "Markdown…",
     "goals": [{"id": 1, "name": "Run a marathon", "description": ""}],
     "objectives": [{"id": 1, "name": "Run 30 km/week", "description": "",
                     "goal": 1, "habits": ["h1"]}]}
  ]
}
```

| Type | Field | Meaning |
| --- | --- | --- |
| `Project` | `id` | What tasks and events refer to |
| | `name` | `/`-separated hierarchy: `School/Calculus` sits under `School` |
| | `color` | `#RRGGBB`, optional |
| | `directory` | Where the project's files are kept |
| | `area` | Life area. Unset, or pointing at a deleted area, means Personal |
| `LifeArea` | `id` | Area `1` is **Personal**. It always exists and cannot be removed, but it can be renamed |
| | `vision` | Markdown |
| | `goals`, `objectives` | Owned by the area |
| `Goal` | `id`, `name`, `description` | Something you aspire to, which can be reached in more than one way |
| `Objective` | `goal` | A goal of the **same** area. Unset or missing means unassigned |
| | `habits` | IDs of the habits that carry it out |

---

## Places and travel

Device-local, in `config.toml` under `[travel]`:

| Field | Meaning |
| --- | --- |
| `api_key` | Mapbox token |
| `home` | Place ID |
| `places` | `[{id, name, address, place_id, lat, lng, favorite}]`. `place_id` is Mapbox's ID (older entries hold a Google ID). `name` can be renamed freely |
| `event_places` | `{event_id: place_id}`. Links a calendar event to a saved place |
| `default_mode` | `walk` / `bike` / `car` |
| `modes` | `[{from, to, mode}]`. Per-trip overrides |
| `routes` | `[{from, to, mode, seconds, meters}]`. Trips that have been calculated |
| `show` | Show travel on the calendar |
| `map_app` | Desktop only: which map to open |

Tasks refer to places with `metadata.location`, and time blocks pass a place
down with `ChildMetadata.location`. The desktop app also caches calculated
trips in the `routes` document (same shape as `routes` above).

---

## Notepad and brain dump

| | Document | Shape |
| --- | --- | --- |
| Notepad | `shared.notepad` | `{"text": "…"}`. The font size (`[notepad] font_size`) stays in `config.toml` per device |
| Brain dump | `shared.brain_dump` | `{"entries": [{"date": "2026-11-09", "text": "…"}]}`. At most one page per day. Never sent to Google |

---

## Work sessions, distractions and pomodoro

Recorded in time-block focus mode. Stored as the `shared.task_work` document
(shared), type `task_work::WorkLog`:

```json
{
  "sessions": [
    {"task_key": "2875093487120931", "task_title": "Write report",
     "block_key": "abc@group…/5ok3…",
     "started_at": "2026-11-09T09:00:00Z", "stopped_at": "2026-11-09T09:25:00Z"}
  ],
  "distractions": [
    {"block_key": "abc@group…/5ok3…", "minutes": 5,
     "recorded_at": "2026-11-09T09:12:00Z"}
  ]
}
```

- `task_key` is `timeblocking::task_key`: the TimeSlay task ID, or
  `<id>/<event id>` for one occurrence of a recurring dated task. Using it
  means the work history survives the task moving between Tasks and Calendar.
- The active session is the most recent one with `stopped_at = null`.
- Distractions are 1, 5, 10, 15 or 30 minutes long.

The pomodoro timer (`[pomodoro]` in `config.toml`, device-local) stores work
and break lengths (25/5 by default), the current phase, the time remaining,
and the task to resume after a break.

---

## Alarms

Stored as the `alarms` document (shared), type `alarms::Library`:

```json
{"alarms": [{
  "id": 1234567890123,
  "at": "2026-11-09T07:30:00+01:00",
  "label": "Leave for the train",
  "profile": {
    "notification": {"enabled": true, "summary": "Time's up", "body": "",
                     "urgency": "critical", "timeout_secs": 0},
    "sound": {"enabled": true, "choice": "chime", "volume": 70,
              "repeats": 2, "fade_in_ms": 150},
    "flash": {"enabled": true, "color": "#FF2020", "peak_opacity": 0.35,
              "pulses": 2, "pulse_ms": 700, "passthrough": false},
    "raise_window": false
  }
}]}
```

- `id` is random, so alarms created offline on different devices cannot
  collide.
- `sound.choice` is a built-in tone (`chime`, `bell`, `beep`, `marimba`,
  `soft_pulse`, `ding`, `ding_dong`, `rise`, `fall`, `kalimba`, `glass`,
  `knock`, `bubble`) or `{"file": "/path/to/sound.ogg"}`.
- `repeats: 0` loops until dismissed. Flash pulses are never shorter than
  334 ms (at most 3 per second), whatever the config says.
- The default profile for new alarms is `[alarm]` in `config.toml`.

---

## Reminders

Notifications for upcoming items are configured per device in `[reminders]`
(`reminders::ReminderSettings`). Reminders are computed, not stored:

| Field | Meaning |
| --- | --- |
| `enabled` | Master switch. Off by default |
| `use_calendar_reminders` | Use each event's Google reminders instead of `events.minutes_before` |
| `events`, `tasks`, `time_blocks`, `habits`, `sleep`, `travel` | `{enabled, minutes_before, sound}` per category |

Each due reminder gets a key that is unique per event, occurrence and lead
time, so it is never delivered twice, and a moved event gets a new reminder.
See [Reminders](./reminders.md).

---

## Notes

Notes are **not in the database**. Each note is `<title>.md` in the folder set
by `[notes] directory`:

```markdown
---
color: rose
links:
  - {"kind":"habit","id":"h1","title":"Read"}
  - {"kind":"event","id":"5ok3…","title":"Standup","day":"2026-11-09"}
tags: [kept, as-is]
---
The note itself, in Markdown.
```

| Front matter | Meaning |
| --- | --- |
| `color` | `butter` (default, omitted), `peach`, `rose`, `lilac`, `lavender`, `sky`, `aqua`, `mint` |
| `links` | Connections: `{kind, id, title, day?}`. `kind` is `event`, `time_block`, `task` or `habit`. The title and day are saved so the link still displays when that day is not loaded. A habit link targets the habit itself, not its events |
| anything else | Kept and written back unchanged |

The file name is the note's identity. Characters not allowed in file names
become `-`, duplicate titles become `Title 2.md`, and an untitled note is
`Untitled.md`. See [Notes](./notes.md).

---

## Contacts

The `contacts` document (device-local) caches what the Google People API
returned, unmodified, for each signed-in account:

```json
{"accounts": [{
  "login": {"label": "me@example.com", "which": "Primary"},
  "groups": [{"resource": "contactGroups/abc", "name": "Family"}],
  "contacts": [ /* raw People API person objects */ ],
  "directory": [ /* Workspace directory results seen so far */ ],
  "no_directory": false
}]}
```

`which` is `"Primary"` (the calendar's sign-in) or `{"Extra": "<email>"}` (an
account connected in Settings). The cache contains no credentials. When an
account is signed out, its entry is removed. Because the raw JSON is kept,
edits can round-trip fields TimeSlay does not display.

---

## Settings (`config.toml`)

Both apps read the same `config.toml`. The Android app uses only a subset
(`core::config::Config`) and ignores the settings it does not know, so you can
copy a desktop config onto a phone as it is. When either app loads the config,
the four shared sections are replaced with the versions from the database.

| Section | Scope | Contents |
| --- | --- | --- |
| `[calendar]` | device | OAuth client, sync interval, hidden calendars, source order, app calendar and task-list IDs, triage requirements, day-end hour, sleep goal and wake hours, first day of week, colors per calendar, list and kind |
| `[projects]` | **shared** via `shared.projects` | See [Life areas…](#life-areas-goals-objectives-and-projects) |
| `[notepad]` | `text` **shared** via `shared.notepad`; `font_size` device | |
| `[brain_dump]` | **shared** via `shared.brain_dump` | |
| `[task_work]` | **shared** via `shared.task_work` | |
| `[travel]` | device | See [Places and travel](#places-and-travel) |
| `[alarm]` | device | Default alarm profile |
| `[reminders]` | device | See [Reminders](#reminders) |
| `[notes]` | device | `directory` |
| `[database_sync]` | device | `url`. Empty disables sync |
| `[appearance]` | device | `theme`, `time_format`, `distance_unit` |
| `[window]`, `[views]`, `[panel]` | desktop | Size, dock and panel geometry, borders, animation, shortcut |
| `[voice]` | desktop | Speech-to-text backend settings |
| `[pomodoro]` | desktop | Timer state |

Sections are left out of the file while they hold only defaults, so an
untouched config stays minimal.

## Secrets

`secrets.json` is a JSON object with one key per kind of credential. Every
write is atomic and done under a file lock. If the file cannot be parsed,
reading it is an error, so nothing can overwrite it.

| Key | Contents |
| --- | --- |
| `google_tokens` | Google OAuth access and refresh tokens (desktop; Android keeps them in its encrypted file) |
| `gmail_accounts` | Extra Google accounts connected for Gmail and People |
| `openrouter_token` | API key for voice and image transcription |

The Slack companion app (`slack/`) keeps its own files,
`.slack-installations.json` and `.slack-app-tokens.json` (mode `0600`). To
add a message to the inbox, it runs `timeslay --webextension`, the same
native-messaging host the browser extension uses. That host forwards the
request to the running app over its IPC socket.

---

## Invariants and gotchas

- **Google is the source of truth** for events and tasks. Rows in SQLite are a
  cache, except while an edit is being sent (the item is locked) and shortly
  after: the `google_entities` overlay wins until a fetch that started after
  Google confirmed the edit.
- **Task IDs never change.** Google IDs do. Use the TimeSlay ID, or a
  `task_key` / block key, for anything that must survive a move.
- **Never overwrite data you could not parse.** This applies to documents,
  task footers, `ttData` and secrets from newer builds: the
  code refuses rather than starting from empty.
- **Shared list items need an identity field** (`id`, `date`, `started_at`,
  `recorded_at` or `drive_id`). Otherwise concurrent edits to the same item
  are kept as two separate items instead of being merged.
- **Empty shared sections are not stored**, so a new device cannot sync
  away existing data.
- **Write documents through `task_db`/`shared_data`.** Direct SQL skips the
  stale-view merge. It is still journaled for sync, but it can overwrite a
  concurrent remote change.
- **The app calendar is identified by name** (`TimeSlay`) until its ID is
  recorded, so a fresh device can recognize it.
