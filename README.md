# TimeSlay

Slay the day with TimeSlay.

There's currently two implementations:

- An [Iced](https://iced.rs) desktop application.
- A [Dioxus](https://dioxuslabs.com) mobile and desktop application.

## Preview

I love docked mode. Your day is always at a glance. You can still hover over, and edit, events and tasks from this mode.

![Dock view](./assets/timeslay-dock.png)

A larger panel to plan and read the day.

![Day view](./assets/timeslay-day.png)

Same for the week. Notice how yesterday can be included in this view.

![Week view](./assets/timeslay-week.png)

## Features

- Stays out of your way during work, but is easily accessible when needed.
	- Toggle between "docked" and "panel" mode as shown above (<kbd>Meta+J</kbd>).
	- Toggle between hiding and showing the app (<kbd>Meta+Shift+J</kbd>).
- Independently reserve screen space for the dock and side panel on Wayland, using the “Reserve screen space” switch in each Settings section. Popups and tooltips do not increase the reservation.
- Intuitive to use (WIP).
- View your time:
	- Day view
	- Week view
	- Month view
	- Custom views for week and month view. (TODO)
		- Based on query; omit private event details. (TODO)
- Manage your time:
	- Events
		- Integrates with meeting software like Zoom and Google Meet.
		- Integrates with guest/event/RSVP going/not going feature (TODO)
		- Uses location data to show estimated travel time for events and to add link to Google Maps (WIP)
	- Time blocks
		- Assign tasks to time blocks.
		- Brain dump.
		- Pomodoro timer in the time block view: configurable work/break lengths, separate elapsed totals, and automatic task stop/resume at phase boundaries. Task tracking also works independently. Pause freezes only the Pomodoro clock; Reset clears its totals. The shared timer continues across blocks and while the popup is closed.
	- Habits
		- Allow "floating" habit, so it can be done anytime during the week/month, etc. or show it on a day +- 3 days etc. Or can make it hard deadline (no +), and have it be greedy (TODO)a
- Manage tasks:
	- Create tasks:
		- With Todoist-like syntax
		- With voice input. (TODO)
		- From Gmail URLs
		- From a web page with the browser extension
		- From a Slack message with the Slack app
	- Triage tasks so they're discoverable later.
	- Workflow to manage tasks not done in days prior (TODO)
	- Workflow to organize/manage pending tasks (TODO)
	- Send low-priority tasks to a backlog.
- Projects (TODO)
	- View the events, tasks, and notes for any project. (TODO)
	- List view, kanban view, calendar view (TODO)
	- During events and tasks associated with a project, easily launch and position the apps/windows necesssary to do the work. (TODO)
- Objectives & Goals (TODO)
	- Tasks, certain amount of time per project. (TODO)
	- Day, week, month, quarter, year. (TODO)
	- SMART goals (TODO)
- Uses Google Calendar, Tasks, and Drive to store events, tasks, and app data.

## Usage

```sh
# Iced desktop app.
cargo run -p time-slay --release

# Dioxus desktop app (minimal).
cargo run -p time-slay-dioxus

# Dioxus mobile app (minimal).
./scripts/build-android.sh
```

### Add links from a browser or Slack

The `time-slay` binary doubles as a native-messaging host (`time-slay
--webextension`), which the browser extension and the Slack app both use to add
the current page or a Slack message to the inbox.

- Browser extension: [`webextension/`](./webextension) — load it unpacked and
  run `./webextension/install.sh` to register the native host.
- Slack app: [`slack/`](./slack) — a socket-mode Bolt app run without a build
  step (`npm start`).

When a TimeSlay window, panel or dock is running it writes the task and shows it
immediately; otherwise the host writes straight to Google Tasks.

## Remote metadata and migration

SQLite remains local (`tasks.sqlite3`), caching task metadata and allocating IDs.
Google is the source of truth after migration: Calendar uses
`extendedProperties.private`; Tasks uses a versioned footer at the end of notes:

```text
normal thing
---
tt-version: 1
tt-id: 100
tt-legacy_key: null
tt-deadline: null
tt-block: null
tt-ignored: false
tt-minutes: 45
tt-order: 0
```

Values are JSON so lists, strings, booleans, and scheduling links round-trip.
Projects, labels, priority, location, and schedule are included when set.
Human notes stay above the separator. Existing work-history keys are preserved.
Task metadata is uploaded on edit and at scheduling checkpoints; fresh syncs
rebuild the SQLite cache from remote metadata instead of overwriting it.
Oversized, malformed, and unsupported-version footers fail without replacement.

**Temporary backwards compatibility:** search for `LEGACY_METADATA_COMPAT`.
On sync, tasks without a footer export their existing SQLite values (or older
`tt-timeblocking:`/`ttid:` notes). Old `DUE:`/`DEADLINE:` lines move into the footer.
Calendar `Type:` footers move into private properties. Failed exports are retried
on the next sync; legacy sources are retained until the remote write succeeds.
Once migration support is no longer needed, remove the marked legacy readers,
SQLite fallback branch in `calendar::task_metadata::load`, and Calendar footer
migration helper. Keep the cache, allocator, aliases, and versioned footer codec.
The shared core crate also supplies these rules to the Dioxus app.

## Cached views and automatic sync

Day, week, and month navigation reads the local cache immediately and never
starts a Google refresh. A request already in progress can fill the cache for
the view it fetched without navigating back or starting another request.
Dates without cached data stay empty until a scheduled or manual refresh.

Automatic refresh defaults to **one hour**. In **Settings → Google Calendar →
Automatic sync**, choose an interval from one minute to 24 hours. Existing saved
intervals are preserved. The last successful refresh time and calendar source
names persist with the event cache, so restarting with fresh data avoids an
immediate refresh. Manual sync and writes you explicitly make still run immediately.
The Dioxus day view follows the same cache and interval policy.

SQLite remains the task metadata cache, ID allocator, and legacy migration
source. Its small schema is retained while migration is supported; event
snapshots continue to use the existing month files. No additional database or
migration is needed for the new refresh policy.

## Next Up

- File attachments/file:// links & notes integration for event/task/habit etc.
- Waiting for reply email (waiting-for tag), once reply, put back in to-do but if not, wait until date to do so. or AI to detect.
- alt+tab proper icon does not show up

### Stretch

- Webhook for Google Calendar
- For everywhere in interface, there should be a new task button
- Command palette
	- Global search
- Reminders (use Google Calendar)
- Logs/instant local field changes/lock field/no waiting for network in interface/show there in queue/undo
- Analytics of time spent
- People/contacts
- Start/shutdown/review rituals for day/week/month
- Offline write queue
- Eisenhower matrix (TODO)
- Theme/project days (TODO)

## Design Constraints

The design of this app is, in part, constrainted by the fact that:

- Google Task API does not allow for reading/writing the time of dday of a due date or deadline (only day).
- Google Task API does not allow writing the deadline field (only due date).
- Google Keep API cannot be used by non-workspace users.
- Integrates only with the KDE Plasma desktop environment on Linux.

Dock and side-panel corner radii can be set independently in Settings. Bind **Toggle Space Reservation** under Keyboard shortcuts to toggle only the currently visible shape; it does nothing while both shapes are hidden. Margin changes update the reserved space live, and popup areas are excluded.
