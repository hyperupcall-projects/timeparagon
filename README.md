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
- Intuitive to use (WIP).
- View and control your time:
	- Day view
	- Week view
	- Month view (TODO)
	- Customized views of day/week/month based on query, if calendar event or details are public (TODO)
	- View and manage events:
		- Events
			- Meetings? Zoom, Google Meet, etc. (TODO)
		- Time blocks
		- Habits
			- Allow "floating" habit, so it can be done anytime during the week/month, etc. or show it on a day +- 3 days etc. Or can make it hard deadline (no +), and have it be greedy
	- Time blocking:
		- Assign tasks to time blocks.
		- Brain dump.
- Manage tasks:
	- Create tasks with Todoist-like syntax and voice input (TODO)
	- Create tasks from URLs and within other apps (Gmail, Slack) (WIP)
	- Triage tasks so they're discoverable later.
	- Workflow to manage tasks not done in days prior (TODO)
	- Send low-priority tasks to a backlog.
- Projects (TODO)
	- View the events, tasks, and notes for any project.
	- List view, kanban view, calendar view.
	- During events and tasks associated with a project, easily launch and position the apps/windows necesssary to do the work. (TODO)
- Objectives & Goals (TODO)
	- Tasks, certain amount of time per project. (TODO)
	- Day, week, month, quarter, year. (TODO)
- Implement popular productivity concepts in the UI:
	- Pomodoro timers (TODO)
	- SMART goals (TODO)
	- Eisenhower matrix (TODO)
	- Brain dumping (TODO)
	- Theme/project days (TODO)
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

## Next Up

- Better UI (shadcdn/pure.css)
- Having the dock take up space on dekstop
- File attachments/file:// links & notes integration for event/task/habit etc.
- Waiting for reply email (waiting-for tag), once reply, put back in to-do but if not, wait until date to do so. or AI to detect.

### Stretch

- For everywhere in interface, there should be a new task button
- Command palette
	- Global search
- Reminders (use Google Calendar)
- Logs/instant local field changes/lock field/no waiting for network in interface/show there in queue/undo
- Analytics of time spent
- People/contacts
- Start/shutdown/review rituals for day/week/month
- Offline write queue

## Design Constraints

The design of this app is, in part, constrainted by the fact that:

- Google Task API does not allow for reading/writing the time of dday of a due date or deadline (only day)
- Google Task API does not allow writing the deadline field (only due date)
- Google Keep API cannot be used by non-workspace users
