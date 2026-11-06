# TimeSlay

The mobile application is built around the day view, with the rest of the app in
sheets over it:

- Adding tasks (with the desktop's quick-add keywords), events and time blocks.
- Notes, with Markdown, colors, connections and Turso sync.
- Search and filters for the day.
- Week, two-week and month agendas.
- Overdue and Inbox filters on the Tasks tab.
- Joining meetings and answering invitations.
- Todos, the notepad, brain dumps and projects, shared through Turso.
- The habit tracker.
- Reminders for events, tasks, time blocks, habits, sleep and travel, with a
  sound per kind (see [docs/reminders.md](../docs/reminders.md)).

Habits, notes, todos, the notepad, brain dumps, projects and work sessions sync
through `https://dragonfruit.tail30229e.ts.net:8210`. Each device must have
Tailscale connected and permission to reach that server. Changes save locally
when offline and retry automatically. Local data stays in the data directory,
because Android may empty an app's cache.

On upgrade, existing records are enrolled transactionally in place. No phone
backup is created. `timeslay.sqlite3` remains the app/widget database, and a
private `turso/replica.db` stores the native sync replica and its sidecars. Never
copy a replica or its sync metadata to another device; enroll that device's own
SQLite database instead. Edits and superseded versions remain in the SQLite
journal and on the server. Export them with:

```sh
cargo run -p timeslay-core --example database_history -- /path/to/timeslay.sqlite3 /new/path/history.json
```

Only one process per device opens the Turso replica. The desktop dock, panel
and Android widget service continue using SQLite; journal triggers capture
changes from every process. Alarms, credentials, theme, calendar/contacts/route
caches and fetch timestamps stay local. Google Calendar and Tasks remain their
own sources of truth. Settings in `config.toml` stay local to each device.

These stay on the desktop: the dock and panel, global shortcuts, alarms and the
Pomodoro timer (neither runs in the background here yet), voice input, People,
habit charts, and the Slack and browser integrations, which reach the phone as
ordinary tasks.

Both apps share data and code via the `timeslay-core` crate:

- the config at `~/.config/timeslay/config.toml`,
- the local database at `~/.local/share/timeslay/timeslay.sqlite3`, which holds
  tasks and time-block metadata, cached events, habits, notes and the scratch data,
- on the desktop, the OAuth token in `~/.local/state/timeslay/secrets.json`,
  kept out of the database,
- the calendar sync and Google Tasks code in `core/`.
- the palettes, radii and shadows in `core/src/theme/`, which reach this app as
  CSS variables through `dioxus/src/style.rs`.

## Usage

Be sure to install the development dependencies:

```sh
sudo apt-get -y install webkit2gtk-4.1 gtk+-3.0 libxdo-dev
```

### Desktop

To test the desktop builds of Dioxus:

```sh
cargo run -p timeslay-dioxus
```

### Android

```sh
./scripts/build-android.sh
```

Useful options:

```sh
# Emulator build
./scripts/build-android.sh --abi x86_64

# Release profile, signed with your own keystore (see below)
TIMESLAY_KEYSTORE_BASE64=$(base64 -w0 ~/keys/timeslay.jks) \
TIMESLAY_KEYSTORE_PASSWORD=... \
TIMESLAY_KEYSTORE_KEYALIAS=timeslay \
./scripts/build-android.sh --release

# Build, install and launch on a connected device
./scripts/build-android.sh --install
```

The APK is copied to `dist/timeslay-<abi>-<profile>.apk`. Its `versionName` is the
version in the root `Cargo.toml`, and its `versionCode` is
`major * 1000000 + minor * 1000 + patch`.

The default (debug) APK is debuggable and debug-signed: use it for development
only. `--release` builds Gradle's release variant, signs it with the keystore
given by `TIMESLAY_KEYSTORE_BASE64` (base64-encoded), `TIMESLAY_KEYSTORE_PASSWORD`
and `TIMESLAY_KEYSTORE_KEYALIAS` (the key uses the keystore password), and fails
unless the result is non-debuggable and not signed with the Android debug key.
Create a keystore once with `keytool -genkeypair -v -keystore
~/keys/timeslay.jks -alias timeslay -keyalg RSA -keysize 4096 -validity
10000` and keep it outside the repository. Passwords must avoid the characters
`" ' \` $ \ & < > =`. The signing block is added to `dioxus/Dioxus.toml` only
while the build runs.

The manifest is `dioxus/AndroidManifest.xml`. It disables Auto Backup. Android
tokens are encrypted with AES-256-GCM using a non-exportable Android Keystore
key. Only ciphertext is stored in `getNoBackupFilesDir()`, which also excludes
tokens from device-to-device transfer independently of manufacturer backup
behavior. Plaintext tokens from older installations are migrated after the
encrypted replacement has been saved, then removed along with old temporary
files. Encryption/decryption failures never fall back to plaintext storage;
if the key becomes unavailable, connect Google again. Desktop token storage
continues to use the existing private token file.

Release verification checks that the APK's signer matches the configured
keystore certificate, and fails if APK inspection or signature verification
fails. Keystore settings must be single-line values. Signing configuration
and generated files are restricted to the build user while building, and
concurrent builds are rejected while signing settings are in use.

### Build requirements

- Android SDK with `platform-tools`, `platforms;android-34` and
  `build-tools;34.0.0`
- An Android NDK (`ANDROID_NDK_HOME`, `NDK_HOME`, or `$ANDROID_HOME/ndk/...`)
- Java 17 to 21 for `--release` (the Android Gradle plugin's lint fails on JDK 24+); any
  Java 17+ for debug builds
- Rust Android targets (installed by the script with `rustup` when needed)
- `dx` from Dioxus CLI 0.7:

  ```sh
  cargo binstall dioxus-cli@0.7.10
  ```

The Android build does **not** compile iced or any desktop-only crates; it uses
`timeslay-core` only.

### Config on the device

Nothing is embedded in the APK: no config, client ID or secret. On first
launch the app asks for the Google OAuth client ID and secret and saves them to
its private storage (`config.toml`, readable only by the app):

```text
/data/user/0/dev.timeslay.day/files/config/timeslay/config.toml
```

After **Save and connect**, sign in with Google in the browser. The rest of the
config (calendars, task lists) is local to this device. Configure it here or
copy the settings from the iced app.

The app sets `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME` to
its Android internal storage before starting, so the shared data layer uses
the same relative layout as it does on desktop.

### Settings and data sync

Turso is required for shared data sync, including notes, habits, todos and
projects. `config.toml` stays local to each device. Both apps watch it and
reload local edits; they do not upload or download settings through Google
Drive, and Google sign-in does not request Drive access.

To remove a previously granted Drive permission, revoke the app's access in
[Google Account connections](https://myaccount.google.com/connections), then
connect Google again in the updated app on each device. Changing the requested
scopes does not revoke existing tokens.

### Mobile layout

At widths of 820px or less, **Calendar** and **Tasks** tabs replace the side-by-side
layout, with Calendar selected by default. The header and tabs stay visible while
the selected pane scrolls independently. All-day events appear in Calendar, and
switching tabs preserves each pane's scroll position. The calendar opens at the
current time (or 8am on another day), and the time-block popup becomes a bottom sheet.

## What the day view shows

- **All-day events** from every visible calendar, in the left rail.
- **Every Google Task for the day** on a visible list, grouped by task list. Completed and
  ignored tasks stay visible with their status, estimate, and a link to the
  time block they belong to.
- **The timed grid** with calendar events and time blocks, using the same
  overlap packing and end-of-day boundary as the iced app.
- A live **current-time needle**, automatic scroll to now, day navigation,
  manual refresh, and a refresh on the configured sync interval.
- Tapping a **time block** opens a popup listing **every task assigned to that
  block**, in planned order, with each task's list, status and estimate, and the
  block's total planned time against its capacity. Tasks can be opened, removed
  and added there, and **Edit block** edits the block itself.

## Editing

Tapping anything else on the day opens a sheet with the same fields as the iced
details pane. It writes through the same code in `timeslay-core`, so both apps
behave alike.

- **Tasks** save each field as it changes: title, description, projects, estimate,
  due date and time, deadline, labels, priority and location. A task with an
  estimate lasts that long, so its time asks only for a start. Complete, ignore,
  triage, move to inbox or backlog, and delete are at the foot of the sheet.
- **Events and time blocks** stage their title, description, times, projects,
  labels, repeat rule, notifications, availability and visibility until **Save**.
  A recurring event can save or delete one occurrence, that one and later ones,
  or the whole series. A block's tasks are laid out again from its new start.
- **Habits** edit like events, keep their length when moved, and can be marked done.
- **Sleep** edits its start, wake-up time and whether each is estimated.
- **Locations** apply at once, from saved places or a Mapbox search. Searching
  needs the Mapbox token from the iced app's settings, which arrives with the
  synced config. A newly chosen place is added to that config too.

The **↻** button lists every habit, whichever day it next falls on, and opens
each in the same sheet. Creating items, and managing the project list, stay in
the iced app.

Time blocks are read from the app's single TimeSlay calendar
(`calendar.app_calendar`, discovered from the calendar list). A block referenced
by a task's metadata is still recognized even if the id changes.

## Home-screen widget (Android)

Add **TimeSlay → Day** from the launcher's widget picker. It shows today's
timeline, scrolled to the current time, with a live now line. Overlapping items
share columns, and when too many overlap for the widget's width the rest collapse
into a **+N** box. A time block lists its tasks beside its title, or under it on a
narrow widget, with unfinished tasks first and **+N more** when they do not fit.
All-day events and tasks without a time or block are chips above the timeline.
Tapping anything, including **+**, opens the app for now.

A widget cannot host the WebView, so it is native Kotlin in `android/`. `dx` has no
option for extra Android sources, so `scripts/build-android.sh` copies them into
its generated project before building. A plain `dx build --android` fails on a
clean tree, because the manifest names the widget's resources.

- `src/widget.rs` writes `files/widget-day.json`. The app does so whenever it shows
  today, and `WidgetRefreshService` syncs about every 30 minutes while it is closed,
  in a separate `:widget` process.
- `DayRenderer.kt` draws the day and hands the widget's `ListView` one bitmap per
  hour, because `RemoteViews` cannot place a view at a time of day on API 24.
- Without Google sign-in nothing is synced, and the widget asks you to open the app.

## Notes

- Only the Dioxus crate enables the desktop/mobile renderers; the root
  workspace defaults to the iced package (`default-members = ["."]`).
- The **☰** button opens the Calendars sheet, which switches Google calendars,
  task lists and TimeSlay event kinds on or off. It saves to this device's
  `[calendar]` settings. Tasks inside a time block's popup stay visible.
- The ⚙ button only edits the Google client ID and secret. Other settings live
  in the iced application; this crate reloads the same config when it changes.
