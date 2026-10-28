# TimeSlay

Slay the day with TimeSlay.

There's currently two implementations:

- An [Iced](https://iced.rs) desktop application
- A [Dioxus](https://dioxuslabs.com) mobile and desktop application

## Screenshots

I love docked mode. Your day is always at a glance. You can still hover over, and edit, events and tasks from this mode.

![Dock view](./assets/timeslay-dock.png)

A larger panel to plan and read the day.

![Day view](./assets/timeslay-day.png)

Same for the week. Notice how yesterday can be included in this view.

![Week view](./assets/timeslay-week.png)

## Features

- Stays out of your way during work, but is easily accessible when needed.
	- Toggle between "docked" and "panel" mode as shown above (<kbd>Meta+Shift+J</kbd>)
	- Toggle between hiding and showing the app (<kbd>Meta+J</kbd>)
- Intuitive to use,
- Uses Google Calendar, Tasks, and Drive to store events, tasks, and app data.

- View and manage events
	- 

## Design Constraints

The design of this app is, in part, constrainted by the fact that:

- Google Task API does not allow for reading/writing the time of dday of a due date or deadline (only day)
- Google Task API does not allow writing the deadline field (only due date)
- Google Keep API cannot be used by non-workspace users

## Usage

```sh
# Iced desktop app.
cargo run -p time-slay --release

# Dioxus desktop app (minimal).
cargo run -p time-slay-dioxus

# Dioxus mobile app (minimal).
./scripts/build-android.sh
```

## Roadmap

- Better handling of recurring tasks
- Better drag and drop
- Better UI
- File attachments
- Light Zoom/Google Meets integration (link parsing)
- More integrations (GMail)
- Actual projects
	- Controlling windows
	- List/kanban/calendar
- Better inbox
	- Input like todoist
	- Natural language add
	- +New button everywhere in interface
- Command palette
	- Global search
- Waiting for reply email (waiting-for tag), once reply, put back in to-do but if not, wait until date to do so. or AI to detect.
- Objects/goals for day/week/mo./quarter/custom
- Reminders (use Google Calendar)

### Stretch

- Logs/instant local field changes/lock field/no waiting for network in interface/show there in queue/undo
- Analytics of time spent
- People/contacts
- Start/shutdown/review rituals for day/week/month
- Saved custom views for daily/weekly/projects?
	- Hide info of "private" calendar events
- Offline write queue
