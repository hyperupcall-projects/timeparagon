# TimeSlay

Slay the day with TimeSlay.

There's currently two implementations:

- An [Iced](https://iced.rs) desktop application
- A [Dioxus](https://dioxuslabs.com) mobile and desktop application

## Screenshots

![Dock view](./assets/timeslay-dock.png)

![Day view](./assets/timeslay-day.png)

![Week view](./assets/timeslay-week.png)

## Features

- Zero-friction, always-on access to app.
	- <kbd>Meta+J</kbd>

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
