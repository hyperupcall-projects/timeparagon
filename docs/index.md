# Documentation

There's currently two implementations of TimeParagon:

- An [Iced](https://iced.rs) desktop application.
- A [Dioxus](https://dioxuslabs.com) mobile application.

Grab a release binary from GitHub or build it yourself:

```sh
# Iced desktop app.
cargo run -p timeparagon --release

# Install Iced desktop app.
./packaging/install.sh

# Dioxus mobile app.
./scripts/build-android.sh

# Dioxus desktop app.
cargo run -p timeparagon-dioxus
```

It is recommended to use [Obtainium](https://github.com/ImranR98/Obtainium) to download the Android app.

CI automatically publishes both the native and Android app when versions are pushed to the repository.

## Setup

To use all features, a bit of setup is required:

- [Google Cloud Account](https://cloud.google.com)
	- Required for syncing calendar, tasks, and contact data.
	- Has usable free tier.
- [Turso](https://turso.tech)
	- Required to sync habit, goal, and all other data including cached calendar, tasks, and contact data.
	- Run `turso --sync-server localhost:<INTERNAL_PORT>`; be careful, only bind to localhost since there's no authentication.
	- Has free tier, but project uses free "development mode" sync server.
- [Tailscale](https://tailscale.com)
	- Highly recommended for creating VPN to connect devices with Turso for syncing.
	- Run it like `sudo tailscale serve --bg --https <TAILNET_PORT> <INTERNAL_PORT>`
	- Has very usable free tier.
- [Syncthing](https://syncthing.net)
	- Highly recommended to sync markdown notes between devices.
	- Completely free.
- Google Accounts
	- Highly recommended to fetch metadata from Gmail URLs.
	- Completely free.
- [Mapbox account](https://mapbox.com)
	- Highly recommended to select locations and calculate travel times.
	- Has usable free tier.
- [OpenRouter](https://openrouter.ai)
	- Optional for translating voice and image input to text and extracting calendar/task metadata from text.
	- Costs money.
- [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
	- Optional for translating voice input to text.
	- If used, recommended to compile from source to use machine GPU/NPU.
	- The `ggml-medium.en-q8_0.bin` [model file](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md) works well for me.
	- Is free, but can be slow, especially on phones.
- [Ollama](https://ollama.com)
	- Optional for translating voice and image input to text and extracting calendar/task metadata from text.
	- Not generally recommended.
	- Is generally free, but can pay for cloud service
- Slack account & Slack app
	- Recommended for adding Slack messages into the inbox.
- Browser extension
	- Recommended for adding website URLs to the inbox
- [Zoom for Google Workspace](https://workspace.google.com/u/0/marketplace/app/zoom_for_google_workspace/364750910244).
	- Highly recommended if you use Zoom.
	- Creating Zoom meetings with API generally not possible, so it opens calendar link in default browser, to be created with Zoom Google Workspace extension

## Philosophy

The app is designed to minimize friction and maximize your time spent in flow state.

In practice, this means that TimeParagon is a [super app](https://en.wikipedia.org/wiki/Super_app) for managing your life. The app:
- Encodes the [most popular](https://www.todoist.com/productivity-methods) [productivity methods](https://www.todoist.com/inspiration/personal-productivity-methods#biological-primetime) in its interface.
- Minimizes context-switching between apps.
- Helps you focus on the work and enter flow state.
- Inspired by [Sunsama](https://www.sunsama.com), [Loop Habits](https://github.com/isoron/uhabits) and [Griply](https://griply.app).

## Architecture

- Desktop app uses [Iced](https://iced.rs) for GUI and mobile app mostly uses [Dioxus](https://dioxuslabs.com).
- Local-first approach; each device has own SQLite database, synced with [Turso](https://turso.tech), which is ran on a separate server.
	- SQLite database contains cache of calendar, task, and other cloud data, and also app-specific data like habit information.
- Uses Google to back calendar, tasks, and contact data.

## Terminology

- Unfinished tasks are tasks, in Google Tasks or on the calendar, not marked as "completed" or "ignored" that are past their deadline or were planned for a day before the current one.
- Inboxed tasks are tasks that the user still needs to organize.

## Design Constraints

The design of this app is, in part, constrainted by the fact that:

- Google Task API does not allow for reading/writing the time of day of a due date or deadline (only day).
- Google Task API does not allow writing the deadline field (only due date).
- Integrates only with the KDE Plasma desktop environment on Linux.
- The Zoom API is typically completely restricted for organization-managed accounts by administrators.

## Features

The documentation below (and pretty  much everywhere else) is AI-generated.

- [Meeetings](./docs/meetings.md)
- [Notes](./docs/notes.md)
- [Reminders](./docs/reminders.md)
- [Routines](./docs/routines.md)
- [Shortcut Daemon](./docs/shortcut-daemon.md)
- [Voice input](./docs/voice-input.md)

## Internals

- [Data model](./docs/data-model.md): what is stored where, identifiers, and the shape of every record.
- [Database schema](./docs/database-schema.md): every SQLite table, trigger and JSON column, plus sync internals.
